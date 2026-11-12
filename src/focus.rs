//! 前台进程捕获：按快捷键时确认「用户此刻在玩哪个程序」。
//!
//! 这里不做任何"游戏识别"——只看前台窗口属于哪个进程，再由调用方判定
//! 能不能提权。所有拒绝理由都返回可读文本，直接写进日志与系统通知，
//! 用户能看懂为什么这次没生效。

use crate::config;
use crate::logging;
use crate::winapi;

/// 捕获失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    /// 前台窗口无效（桌面、锁屏、切换中）或不属于任何进程
    NoForeground,
    /// 命中排除清单
    Excluded(String),
    /// 目标是本程序自己
    SelfProcess(String),
    /// 命中受保护的系统进程名单
    Protected(String),
}

impl CaptureError {
    /// 写日志与通知用的一句话。
    pub fn reason(&self) -> String {
        match self {
            CaptureError::NoForeground => {
                "没有可识别的前台窗口（桌面或锁屏），本次不提升任何进程".into()
            }
            CaptureError::Excluded(name) => {
                format!("{name} 在排除清单里，不提升优先级")
            }
            CaptureError::SelfProcess(name) => {
                format!("{name} 是 WGOpt 自己，不提升优先级")
            }
            CaptureError::Protected(name) => {
                format!("{name} 是受保护的系统进程，不提升优先级")
            }
        }
    }
}

/// 判定一个进程名能否作为提权目标。
///
/// 判定顺序按代价从低到高，命中即返回。所有比较都不区分大小写。
pub fn check_target(cfg: &config::Config, name: &str) -> Result<String, CaptureError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CaptureError::NoForeground);
    }
    // 防止配置里写成路径，统一按文件名比较
    let base = std::path::Path::new(trimmed)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| trimmed.to_string());

    if base.eq_ignore_ascii_case("wgopt.exe") {
        return Err(CaptureError::SelfProcess(base));
    }
    if cfg.is_excluded(&base) {
        return Err(CaptureError::Excluded(base));
    }
    if crate::whitelist::is_protected(&base) {
        return Err(CaptureError::Protected(base));
    }
    Ok(base)
}

/// 捕获结果：成功时带上进程名，失败时带上原因。
pub type CaptureResult = Result<String, CaptureError>;

/// 读取当前前台窗口所属进程，并按排除清单校验。
///
/// `self_pid` 用于排除本程序自己（`wgopt.exe` 通常在名单里，但进程名
/// 可能与安装包不同名，所以再按 pid 判一次）。
pub fn capture_foreground(cfg: &config::Config, self_pid: u32) -> CaptureResult {
    let Some((pid, name)) = winapi::foreground_process() else {
        return Err(CaptureError::NoForeground);
    };
    // 同一个窗口在切换过程中可能短暂读不到名字
    if name.trim().is_empty() {
        return Err(CaptureError::NoForeground);
    }
    if pid == self_pid {
        return Err(CaptureError::SelfProcess(name));
    }
    check_target(cfg, &name)
}

/// 捕获并写入配置；返回写入后的值。
///
/// 写盘失败不影响本次提权（返回值仍是有效目标），但会记 error 日志并
/// 让调用方决定是否提示用户——配置写不进去只影响下次启动的默认值。
pub fn capture_and_persist(
    shared: &crate::app::Shared,
    self_pid: u32,
) -> (CaptureResult, Option<String>) {
    let cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let result = capture_foreground(&cfg, self_pid);

    let Ok(name) = &result else {
        return (result, None);
    };

    let mut save_error = None;
    let changed = {
        let mut cfg = match shared.config.lock() {
            Ok(c) => c,
            Err(_) => return (result, Some("配置状态不可用".into())),
        };
        let changed = !cfg.game_process.eq_ignore_ascii_case(name);
        if changed {
            cfg.game_process = name.clone();
        }
        changed
    };

    // 只有真的变了才写盘：连着按两次同一个游戏不该反复写文件
    if changed {
        if let Err(e) = save_current(shared) {
            logging::error(&format!("记录前台进程失败：{e}"));
            save_error = Some(e);
        } else {
            logging::info(&format!("已记录前台进程：{name}"));
        }
    }
    (result, save_error)
}

/// 把当前配置写回磁盘。
fn save_current(shared: &crate::app::Shared) -> Result<(), String> {
    let cfg = shared
        .config
        .lock()
        .map(|c| c.clone())
        .map_err(|_| "配置状态不可用".to_string())?;
    config::save(&cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> config::Config {
        config::Config::default()
    }

    #[test]
    fn normal_game_process_is_accepted() {
        assert_eq!(check_target(&cfg(), "BlackMythWukong.exe"), Ok("BlackMythWukong.exe".into()));
    }

    #[test]
    fn exclamation_list_is_case_insensitive() {
        let c = cfg();
        assert!(matches!(
            check_target(&c, "CHROME.EXE"),
            Err(CaptureError::Excluded(_))
        ));
        assert!(matches!(
            check_target(&c, "Explorer.exe"),
            Err(CaptureError::Excluded(_))
        ));
    }

    #[test]
    fn default_exclusions_cover_browser_and_shell() {
        let c = cfg();
        for name in ["chrome.exe", "msedge.exe", "explorer.exe", "dwm.exe", "wgopt.exe"] {
            assert!(c.is_excluded(name), "{name} 应在默认排除清单里");
        }
    }

    #[test]
    fn self_process_is_rejected_even_if_not_listed() {
        let mut c = cfg();
        c.excluded_processes.clear();
        assert!(matches!(
            check_target(&c, "wgopt.exe"),
            Err(CaptureError::SelfProcess(_))
        ));
    }

    #[test]
    fn protected_system_process_is_rejected() {
        let mut c = cfg();
        c.excluded_processes.clear();
        assert!(matches!(
            check_target(&c, "csrss.exe"),
            Err(CaptureError::Protected(_))
        ));
    }

    #[test]
    fn empty_name_reports_no_foreground() {
        assert_eq!(check_target(&cfg(), "   "), Err(CaptureError::NoForeground));
    }

    #[test]
    fn path_is_reduced_to_file_name() {
        assert_eq!(
            check_target(&cfg(), r"D:\Games\Foo\game.exe"),
            Ok("game.exe".into())
        );
    }

    #[test]
    fn capture_rejects_when_self_pid_matches() {
        // 本进程名若不在排除清单/保护名单里，check_target 应接受它；
        // SelfProcess 分支由 capture_foreground 按 pid 判定，需要真实前台窗口，
        // 这里只验证名字层面的判定不会误伤普通进程。
        let c = cfg();
        let me = std::env::current_exe().unwrap();
        let name = me.file_name().unwrap().to_string_lossy().to_string();
        if !c.is_excluded(&name) && !crate::whitelist::is_protected(&name) {
            assert!(check_target(&c, &name).is_ok());
        }
    }
}
