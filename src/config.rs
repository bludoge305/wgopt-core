//! 配置持久化：`%APPDATA%\WGOpt\config.json`。
//!
//! 与原版共用同一路径与字段名，升级不丢旧配置。原版只把界面相关字段
//! 放在这里，本版本（无界面）额外补充核心模式需要的开关。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::item;

/// 配置文件名。
pub const FILE_NAME: &str = "config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 优化总开关。关闭时后台常驻但不做任何优化，托盘菜单可切换。
    pub enabled: bool,
    /// 各优化项的勾选状态，key 为 item id
    pub enabled_items: Vec<String>,
    /// 开机自启
    pub autostart: bool,
    /// 开始优化时弹出系统通知
    pub notify_on_start: bool,
    /// 停止优化并还原时弹出系统通知
    pub notify_on_stop: bool,
    /// 程序退出时还原可还原项（进程优先级、服务、被关闭的 Wallpaper Engine）
    pub restore_on_exit: bool,
    /// 全局快捷键开关：关闭后不再注册快捷键
    pub hotkey_enabled: bool,
    /// 「立即开始优化」的全局快捷键，如 `Ctrl+F2`
    pub hotkey: String,
    /// 最近一次按快捷键时自动识别到的前台进程名（如 `game.exe`）。
    ///
    /// 快捷键触发时写入，之后的优化轮次优先使用它。为空表示还没识别过。
    pub game_process: String,
    /// 绝不允许提升优先级的进程名清单（小写、含 `.exe`）。
    ///
    /// 即使某个进程此刻是前台窗口，命中这里也不会被提升。
    pub excluded_processes: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_enabled_items() -> Vec<String> {
    item::item_definitions().into_iter().map(|i| i.id).collect()
}

fn default_hotkey() -> String {
    crate::hotkey::DEFAULT_HOTKEY.to_string()
}

/// 默认排除清单：这些进程即使占据前台窗口也绝不提升优先级。
///
/// 覆盖三类必须排除的情况：
/// 1. 系统外壳与关键服务——给它们提高优先级会拖慢或卡死整个系统
/// 2. 浏览器与常见桌面程序——用户在看网页/聊天时按快捷键不该把它们提权
/// 3. 本程序自身
pub fn default_excluded_processes() -> Vec<String> {
    [
        // 系统外壳与关键进程
        "explorer.exe",
        "dwm.exe",
        "csrss.exe",
        "wininit.exe",
        "winlogon.exe",
        "services.exe",
        "lsass.exe",
        "smss.exe",
        "svchost.exe",
        "taskhostw.exe",
        "sihost.exe",
        "ctfmon.exe",
        "runtimebroker.exe",
        "fontdrvhost.exe",
        "audiodg.exe",
        "searchhost.exe",
        "startmenuexperiencehost.exe",
        "shellexperiencehost.exe",
        "applicationframehost.exe",
        "textinputhost.exe",
        "lockapp.exe",
        // 浏览器
        "chrome.exe",
        "msedge.exe",
        "firefox.exe",
        "brave.exe",
        "opera.exe",
        "vivaldi.exe",
        "iexplore.exe",
        "chromium.exe",
        "zen.exe",
        // 开发与终端
        "code.exe",
        "devenv.exe",
        "pycharm64.exe",
        "idea64.exe",
        "windowsterminal.exe",
        "conhost.exe",
        "cmd.exe",
        "powershell.exe",
        "pwsh.exe",
        "wsl.exe",
        "wslhost.exe",
        // 通讯与办公
        "wechat.exe",
        "weixin.exe",
        "qq.exe",
        "dingtalk.exe",
        "feishu.exe",
        "lark.exe",
        "teams.exe",
        "slack.exe",
        "discord.exe",
        "telegram.exe",
        "winword.exe",
        "excel.exe",
        "powerpnt.exe",
        "wps.exe",
        "notion.exe",
        "obsidian.exe",
        "typora.exe",
        // 媒体
        "vlc.exe",
        "potplayermini64.exe",
        "mpc-hc64.exe",
        "wmplayer.exe",
        "obs64.exe",
        "obs32.exe",
        // 本程序
        "wgopt.exe",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            enabled_items: default_enabled_items(),
            autostart: false,
            notify_on_start: default_true(),
            notify_on_stop: default_true(),
            restore_on_exit: default_true(),
            hotkey_enabled: true,
            hotkey: default_hotkey(),
            game_process: String::new(),
            excluded_processes: default_excluded_processes(),
        }
    }
}

impl Config {
    /// 归一化：去掉不存在的优化项 id，全部被删光时回退到默认。
    pub fn normalize(&mut self) {
        let known: Vec<String> = item::item_definitions().into_iter().map(|i| i.id).collect();
        self.enabled_items
            .retain(|id| known.iter().any(|k| k == id));
        self.enabled_items.dedup();
        if self.enabled_items.is_empty() {
            self.enabled_items = known;
        }

        // 排除清单统一小写去重：匹配一律不走大小写敏感路径
        self.excluded_processes
            .retain(|n| !n.trim().is_empty());
        for n in &mut self.excluded_processes {
            *n = n.trim().to_ascii_lowercase();
        }
        self.excluded_processes.sort();
        self.excluded_processes.dedup();

        // 记住的进程名不要带路径，只留文件名
        self.game_process = file_name(&self.game_process);
    }

    /// 某个进程是否在排除清单里（大小写不敏感）。
    ///
    /// 命中这里的进程即使此刻占据前台窗口也不会被提升优先级。
    pub fn is_excluded(&self, name: &str) -> bool {
        let lower = name.trim().to_ascii_lowercase();
        self.excluded_processes.contains(&lower)
    }

    /// 某项是否启用。
    pub fn is_item_enabled(&self, id: &str) -> bool {
        self.enabled_items.iter().any(|x| x == id)
    }

    pub fn set_item_enabled(&mut self, id: &str, on: bool) {
        let has = self.is_item_enabled(id);
        if on && !has {
            // 按定义顺序插入，保持配置文件可读
            let mut ids: Vec<String> = Vec::new();
            for def in item::item_definitions() {
                if def.id == id || self.is_item_enabled(&def.id) {
                    ids.push(def.id);
                }
            }
            self.enabled_items = ids;
        } else if !on && has {
            self.enabled_items.retain(|x| x != id);
        }
    }

    /// 解析生效的快捷键。
    ///
    /// 开关关闭返回 `Ok(None)`；格式非法返回 `Err` 而不是静默失败——
    /// 非法配置只影响快捷键本身，不改变「写坏配置按停用处理」那条约束。
    pub fn active_hotkey(&self) -> Result<Option<crate::hotkey::Hotkey>, String> {
        if !self.hotkey_enabled {
            return Ok(None);
        }
        crate::hotkey::Hotkey::parse(&self.hotkey).map(Some)
    }
}

/// 取路径中的文件名；空串或纯路径保持为空。
fn file_name(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    std::path::Path::new(trimmed)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| trimmed.to_string())
}

/// 配置目录：`%APPDATA%\WGOpt`，与原版 Tauri 的 app_config_dir 一致，
/// 因此从桌面版切过来不会丢配置。
pub fn config_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("WGOpt");
    }
    PathBuf::from(".")
}

pub fn config_path() -> PathBuf {
    config_dir().join(FILE_NAME)
}

/// 从任意目录读取配置。
///
/// 文件不存在时用默认值（首次运行）；文件存在但解析失败时**按停用处理**
/// 并保留原文件不动——写坏的配置若静默回退成"全开"，后台进程会在用户
/// 不知情的情况下继续结束进程、停服务。宁可不动，也不要误伤。
pub fn load_from_dir(dir: &Path) -> Config {
    let path = dir.join(FILE_NAME);
    let Ok(text) = fs::read_to_string(&path) else {
        return Config::default();
    };
    // 去 BOM：手工编辑过的文件常带 BOM，serde 会因此解析失败
    let text = text.trim_start_matches('\u{feff}');
    match serde_json::from_str::<Config>(text) {
        Ok(mut cfg) => {
            cfg.normalize();
            cfg
        }
        Err(e) => {
            crate::logging::error(&format!(
                "配置文件解析失败（{}），已按「总开关关闭」处理：{e}",
                path.display()
            ));
            Config {
                enabled: false,
                ..Config::default()
            }
        }
    }
}

/// 读取默认目录下的配置。
pub fn load() -> Config {
    load_from_dir(&config_dir())
}

/// 保存到任意目录。
pub fn save_to_dir(dir: &Path, config: &Config) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let text = serde_json::to_string_pretty(config).map_err(|e| format!("序列化配置失败：{e}"))?;
    fs::write(dir.join(FILE_NAME), text).map_err(|e| format!("写入配置失败：{e}"))
}

/// 保存到默认目录。
pub fn save(config: &Config) -> Result<(), String> {
    save_to_dir(&config_dir(), config)
}

// ---------------------------------------------------------------- 开机自启

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "WGOpt";

/// 设置或取消开机自启（写入 HKCU，无需管理员）。
#[cfg(windows)]
pub fn set_autostart(enable: bool) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_SZ,
    };

    let wkey: Vec<u16> = RUN_KEY.encode_utf16().chain(std::iter::once(0)).collect();
    let wvalue: Vec<u16> = RUN_VALUE.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let mut hkey = HKEY::default();
        let res = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wkey.as_ptr()),
            Some(0),
            KEY_SET_VALUE,
            &mut hkey,
        );
        if res != ERROR_SUCCESS {
            return Err(format!("打开注册表失败（错误码 {}）", res.0));
        }

        let result = if enable {
            let exe = std::env::current_exe().map_err(|e| format!("获取程序路径失败：{e}"))?;
            let value = format!("\"{}\" --autostart", exe.display());
            let wdata: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = std::slice::from_raw_parts(
                wdata.as_ptr() as *const u8,
                wdata.len() * std::mem::size_of::<u16>(),
            );
            RegSetValueExW(hkey, PCWSTR(wvalue.as_ptr()), Some(0), REG_SZ, Some(bytes))
        } else {
            // 值不存在同样视为成功
            let _ = RegDeleteValueW(hkey, PCWSTR(wvalue.as_ptr()));
            ERROR_SUCCESS
        };

        let _ = RegCloseKey(hkey);

        if result == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("写入注册表失败（错误码 {}）", result.0))
        }
    }
}

#[cfg(not(windows))]
pub fn set_autostart(_enable: bool) -> Result<(), String> {
    Err("当前平台不支持".into())
}

/// 查询是否已设置开机自启。
#[cfg(windows)]
pub fn is_autostart_enabled() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
    };

    let wkey: Vec<u16> = RUN_KEY.encode_utf16().chain(std::iter::once(0)).collect();
    let wvalue: Vec<u16> = RUN_VALUE.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wkey.as_ptr()),
            Some(0),
            KEY_QUERY_VALUE,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return false;
        }
        let mut buf = vec![0u8; 2048];
        let mut size = buf.len() as u32;
        let res = RegQueryValueExW(
            hkey,
            PCWSTR(wvalue.as_ptr()),
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);
        res == ERROR_SUCCESS
    }
}

#[cfg(not(windows))]
pub fn is_autostart_enabled() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_enables_all_items() {
        let cfg = Config::default();
        assert_eq!(cfg.enabled_items.len(), item::item_definitions().len());
        assert!(cfg.enabled);
    }

    #[test]
    fn normalize_drops_unknown_ids() {
        let mut cfg = Config {
            enabled_items: vec!["memory".into(), "nope".into()],
            ..Default::default()
        };
        cfg.normalize();
        assert_eq!(cfg.enabled_items, vec!["memory".to_string()]);
    }

    #[test]
    fn normalize_restores_defaults_when_all_dropped() {
        let mut cfg = Config {
            enabled_items: vec!["nope".into()],
            ..Default::default()
        };
        cfg.normalize();
        assert_eq!(cfg.enabled_items.len(), item::item_definitions().len());
    }

    #[test]
    fn set_item_enabled_keeps_definition_order() {
        let mut cfg = Config {
            enabled_items: vec!["memory".into(), "dns".into()],
            ..Default::default()
        };
        cfg.set_item_enabled("clipboard", true);
        cfg.set_item_enabled("dns", false);
        assert_eq!(
            cfg.enabled_items,
            vec!["memory".to_string(), "clipboard".to_string()]
        );
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = std::env::temp_dir().join("wgopt-test-missing");
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = load_from_dir(&dir);
        assert!(cfg.enabled);
        assert_eq!(cfg.enabled_items.len(), item::item_definitions().len());
    }

    #[test]
    fn load_tolerates_bom() {
        let dir = std::env::temp_dir().join("wgopt-test-bom");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);

        std::fs::write(&path, "\u{feff}{\"enabled_items\":[\"dns\"]}").unwrap();
        assert_eq!(load_from_dir(&dir).enabled_items, vec!["dns".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 写坏的配置必须按「停用」处理：静默回退成默认全开会让后台进程
    /// 在用户不知情时继续结束进程、停服务。
    #[test]
    fn broken_json_fails_safe_to_disabled() {
        let dir = std::env::temp_dir().join("wgopt-test-broken");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);

        std::fs::write(&path, "{ not json").unwrap();
        let cfg = load_from_dir(&dir);
        assert!(!cfg.enabled, "损坏的配置必须让总开关关闭");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_hotkey_is_ctrl_f2_and_enabled() {
        let cfg = Config::default();
        assert!(cfg.hotkey_enabled);
        let hk = cfg.active_hotkey().unwrap().expect("默认应返回组合键");
        assert_eq!(hk.to_string(), "Ctrl+F2");
    }

    /// 旧配置文件没有这两个字段时按默认补齐，且不影响总开关的判定。
    #[test]
    fn old_config_without_hotkey_gets_default() {
        let dir = std::env::temp_dir().join("wgopt-test-old-hotkey");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE_NAME), r#"{"enabled":true}"#).unwrap();

        let cfg = load_from_dir(&dir);
        assert!(cfg.enabled);
        assert!(cfg.hotkey_enabled);
        assert_eq!(cfg.hotkey, crate::hotkey::DEFAULT_HOTKEY);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hotkey_switch_off_yields_none() {
        let cfg = Config {
            hotkey_enabled: false,
            ..Default::default()
        };
        assert!(cfg.active_hotkey().unwrap().is_none());
    }

    /// 快捷键写错只报错，不能让整个配置按停用处理——否则用户改错一个
    /// 按键名，后台就不再优化了。
    #[test]
    fn invalid_hotkey_keeps_config_usable() {
        let dir = std::env::temp_dir().join("wgopt-test-bad-hotkey");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(FILE_NAME),
            r#"{"enabled":true,"hotkey":"Ctrl+NotAKey"}"#,
        )
        .unwrap();

        let cfg = load_from_dir(&dir);
        assert!(cfg.enabled, "按键写错不影响总开关");
        assert!(cfg.active_hotkey().is_err(), "非法组合键要能被识别");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_hotkey_roundtrips() {
        let cfg = Config {
            hotkey: "Alt+Shift+F9".into(),
            ..Default::default()
        };
        assert_eq!(cfg.active_hotkey().unwrap().unwrap().to_string(), "Alt+Shift+F9");
    }

    /// 默认排除清单必须覆盖解释器类系统进程，否则按快捷键会把它们提权。
    #[test]
    fn default_exclusions_cover_shell_and_browsers() {
        let cfg = Config::default();
        assert!(!cfg.excluded_processes.is_empty());
        for name in ["explorer.exe", "chrome.exe", "dwm.exe", "wgopt.exe"] {
            assert!(cfg.is_excluded(name), "{name} 应在默认排除清单里");
        }
        // 常见游戏不在排除清单里，否则识别机制就没法工作
        assert!(!cfg.is_excluded("blackmythwukong.exe"));
    }

    /// 归一化把排除清单统一成小写并去重，匹配时不区分用户写法。
    #[test]
    fn normalize_lowercases_and_dedups_exclusions() {
        let mut cfg = Config {
            excluded_processes: vec!["Chrome.EXE".into(), " chrome.exe ".into(), "".into()],
            ..Default::default()
        };
        cfg.normalize();
        assert_eq!(cfg.excluded_processes, vec!["chrome.exe".to_string()]);
        assert!(cfg.is_excluded("CHROME.EXE"));
    }

    /// game_process 只保留文件名，手工写成路径也不会带进去。
    #[test]
    fn normalize_strips_path_from_game_process() {
        let mut cfg = Config {
            game_process: r"D:\Games\Foo\game.exe".into(),
            ..Default::default()
        };
        cfg.normalize();
        assert_eq!(cfg.game_process, "game.exe");
    }

    /// 旧配置没有新字段时按默认补齐，且能正常读取。
    #[test]
    fn old_config_without_new_fields_gets_defaults() {
        let dir = std::env::temp_dir().join("wgopt-test-new-fields");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE_NAME), r#"{"enabled":true}"#).unwrap();

        let cfg = load_from_dir(&dir);
        assert!(cfg.enabled);
        assert!(cfg.game_process.is_empty());
        assert!(!cfg.excluded_processes.is_empty(), "缺省应补上默认排除清单");
        assert!(cfg.is_excluded("chrome.exe"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
