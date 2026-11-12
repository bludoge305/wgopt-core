//! 托盘图标：状态显示、右键菜单、气泡通知。
//!
//! 用 Shell_NotifyIconW 直接实现，不引入 GUI 框架——本程序的窗口是一条
//! 不可见的消息窗口，只负责收托盘事件。

use std::path::PathBuf;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO,
    NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, DestroyIcon, GetCursorPos, LoadImageW,
    SetForegroundWindow, TrackPopupMenu, HICON, HMENU, IMAGE_ICON, LR_LOADFROMFILE, MF_CHECKED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP,
};

use crate::app::ToggleAction;
use crate::winapi::wide;

/// 托盘回调消息（WM_APP + 1）。
pub const WM_TRAY: u32 = WM_APP + 1;

/// 托盘图标 ID。
const TRAY_ID: u32 = 1;

/// 三态图标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIcon {
    /// 待机：没有正在生效的优化
    Idle,
    /// 优化生效中
    Active,
    /// 总开关已关闭
    Off,
}

impl TrayIcon {
    fn asset(&self) -> &'static str {
        match self {
            TrayIcon::Idle => "idle.ico",
            TrayIcon::Active => "active.ico",
            TrayIcon::Off => "off.ico",
        }
    }
}

/// 通知类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyKind {
    Info,
    Warn,
    Error,
}

/// 菜单命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    ToggleEnabled,
    /// 优化状态切换：未优化时开始、已优化时还原。
    ///
    /// 具体走哪个方向由 `app::Shared::toggle_action` 判定，不在菜单层判断。
    ToggleRun,
    ToggleAutostart,
    ToggleHotkey,
    OpenConfig,
    Quit,
}

/// 菜单选择结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuChoice {
    Command(Command),
    /// 切换第 index 个优化项（对应 `item_definitions()` 的顺序）
    ToggleItem(usize),
}

/// 菜单渲染所需的状态快照。
#[derive(Debug, Clone)]
pub struct MenuState {
    pub enabled: bool,
    pub autostart: bool,
    /// 优化状态切换项：文案与可用性都取自同一个判定
    pub toggle: ToggleAction,
    /// 全局快捷键开关（已实际注册成功）
    pub hotkey_enabled: bool,
    /// 快捷键文本：注册成功时是组合键，否则回退到配置里的写法
    pub hotkey_label: String,
    /// 顶部状态文字
    pub status_line: String,
    /// 与 `item_definitions()` 顺序一致的各优化项开关
    pub items: Vec<bool>,
}

const ID_TOGGLE_ENABLED: u32 = 1;
/// 优化状态切换项（开始/还原共用）。
const ID_TOGGLE_RUN: u32 = 3;
/// 原「立即开始优化」的 id，与切换项合并后不再使用；保留常量避免 id 复用冲突。
const ID_TOGGLE_AUTOSTART: u32 = 5;
const ID_OPEN_CONFIG: u32 = 6;
const ID_QUIT: u32 = 7;
const ID_TOGGLE_HOTKEY: u32 = 8;
/// 优化项菜单 ID 基址，避免与上面的固定 ID 冲突。
const MENU_ITEM_BASE: u32 = 100;

/// 托盘对象：持有消息窗口句柄与图标资源。
pub struct Tray {
    hwnd: HWND,
    current: Option<TrayIcon>,
    last_tip: String,
    icons: Vec<(TrayIcon, HICON)>,
    /// 图标当前是否真的挂在通知区域。add 失败或 Explorer 重启后为 false，
    /// 下一次 update 会改用 NIM_ADD 重新注册。
    attached: bool,
}

impl Tray {
    /// 绑定到消息窗口。`icon_dir` 是存放三个 ico 的目录。
    pub fn new(hwnd: HWND, icon_dir: PathBuf) -> Self {
        let mut icons = Vec::new();
        for icon in [TrayIcon::Idle, TrayIcon::Active, TrayIcon::Off] {
            if let Some(h) = load_icon(&icon_dir.join(icon.asset())) {
                icons.push((icon, h));
            }
        }
        Self {
            hwnd,
            current: None,
            last_tip: String::new(),
            icons,
            attached: false,
        }
    }

    /// 图标目录：程序同目录的 assets\tray，其次配置目录下的 icons。
    pub fn resolve_icon_dir() -> PathBuf {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_default();
        let candidates = [
            exe_dir.join("tray"),
            exe_dir.join("assets").join("tray"),
            crate::config::config_dir().join("icons"),
        ];
        candidates
            .into_iter()
            .find(|d| d.join(TrayIcon::Idle.asset()).exists())
            .unwrap_or_else(|| exe_dir.join("tray"))
    }

    fn icon_for(&self, kind: TrayIcon) -> HICON {
        self.icons
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, h)| *h)
            .unwrap_or_default()
    }

    fn base_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            ..Default::default()
        }
    }

    /// 首次添加图标。返回是否成功挂到通知区域。
    pub fn add(&mut self, icon: TrayIcon, tip: &str) -> bool {
        let mut data = self.base_data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = self.icon_for(icon);
        set_text(&mut data.szTip, tip);
        self.attached = unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool();
        self.current = Some(icon);
        self.last_tip = tip.to_string();
        self.attached
    }

    /// 更新图标与提示文字；两者都变时才写托盘，避免托盘闪烁。
    ///
    /// 图标没挂上时（首次 add 失败、Explorer 重启过）改用 `NIM_ADD` 重挂，
    /// 否则 `NIM_MODIFY` 会一直失败，托盘永远不再出现。
    pub fn update(&mut self, icon: TrayIcon, tip: &str) {
        if !self.attached {
            self.add(icon, tip);
            return;
        }
        let changed_icon = self.current != Some(icon);
        if !changed_icon && self.last_tip == tip {
            return;
        }
        let mut data = self.base_data();
        data.uFlags = if changed_icon {
            NIF_MESSAGE | NIF_ICON | NIF_TIP
        } else {
            NIF_TIP
        };
        data.uCallbackMessage = WM_TRAY;
        if changed_icon {
            data.hIcon = self.icon_for(icon);
        }
        set_text(&mut data.szTip, tip);
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
            self.current = Some(icon);
            self.last_tip = tip.to_string();
        } else {
            // 写失败说明图标已不在通知区域（典型是 Explorer 重启），
            // 下一拍重挂
            self.attached = false;
        }
    }

    /// 弹出气泡通知。
    pub fn notify(&self, title: &str, body: &str, kind: NotifyKind) {
        let mut data = self.base_data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = match kind {
            NotifyKind::Info => NIIF_INFO,
            NotifyKind::Warn => NIIF_WARNING,
            NotifyKind::Error => NIIF_ERROR,
        };
        set_text(&mut data.szInfoTitle, title);
        set_text(&mut data.szInfo, body);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    /// Explorer 重启后托盘图标会消失，重新注册一次。
    ///
    /// `NIM_ADD` 对同一 (hwnd, id) 是幂等的：Explorer 还活着时等于刷新，
    /// 重启过则重新挂上。失败也不要紧，`update` 下一拍会再补。
    pub fn reattach(&mut self) {
        let icon = self.current.unwrap_or(TrayIcon::Idle);
        let tip = if self.last_tip.is_empty() {
            "WGOpt".to_string()
        } else {
            self.last_tip.clone()
        };
        self.add(icon, &tip);
    }

    /// 弹出右键菜单并返回用户选择。
    pub fn popup_menu(&self, state: &MenuState) -> Option<MenuChoice> {
        let menu = unsafe { CreatePopupMenu().ok()? };
        build_menu(menu, state);

        let mut pt = windows::Win32::Foundation::POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
            // 必须先置前台，否则菜单点外部不会消失
            let _ = SetForegroundWindow(self.hwnd);
        }

        let flags = TPM_RETURNCMD | TPM_LEFTALIGN | TPM_RIGHTBUTTON;
        let picked = unsafe { TrackPopupMenu(menu, flags, pt.x, pt.y, None, self.hwnd, None) };
        unsafe {
            let _ = DestroyMenu(menu);
        }
        translate_choice(picked.0 as u32)
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let mut data = self.base_data();
            data.uFlags = NIF_MESSAGE;
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            for (_, h) in self.icons.drain(..) {
                if !h.is_invalid() {
                    let _ = DestroyIcon(h);
                }
            }
        }
    }
}

fn menu_str(menu: HMENU, flags: windows::Win32::UI::WindowsAndMessaging::MENU_ITEM_FLAGS, id: u32, text: &str) {
    let w = wide(text);
    unsafe {
        let _ = AppendMenuW(menu, flags, id as usize, PCWSTR(w.as_ptr()));
    }
}

/// 优化状态切换项的菜单文字。
///
/// 文案取自 `ToggleAction::label`，与快捷键执行的动作是同一个判定；
/// 快捷键按惯例用 `\t` 隔开、由系统右对齐显示。
fn toggle_label(state: &MenuState) -> String {
    let base = state.toggle.label();
    match (state.hotkey_enabled, state.hotkey_label.is_empty()) {
        (true, false) => format!("{base}\t{}", state.hotkey_label),
        _ => base.to_string(),
    }
}

/// 「启用快捷键」的菜单文字。
///
/// 关闭状态下仍显示按键，用户能看到重新打开后会绑什么键；配置写坏时
/// `hotkey_label` 为空，只剩开关名，不会显示一个用不了的组合键。
fn hotkey_label(state: &MenuState) -> String {
    if state.hotkey_label.is_empty() {
        "启用快捷键".to_string()
    } else {
        format!("启用快捷键（{}）", state.hotkey_label)
    }
}

fn build_menu(menu: HMENU, state: &MenuState) {
    // 顶部状态行不可点击
    menu_str(menu, MF_STRING | MF_GRAYED, 0, &state.status_line);
    menu_str(menu, MF_SEPARATOR, 0, "");

    menu_str(
        menu,
        MF_STRING | if state.enabled { MF_CHECKED } else { MF_STRING },
        ID_TOGGLE_ENABLED,
        "启用优化总开关",
    );

    menu_str(menu, MF_SEPARATOR, 0, "");

    // 开始与还原是同一个开关的两个方向，共用一个菜单项
    let toggle_flags = if state.toggle.is_available() {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };
    menu_str(menu, toggle_flags, ID_TOGGLE_RUN, &toggle_label(state));

    // 快捷键开关与切换项同属触发方式，紧挨着放
    menu_str(
        menu,
        MF_STRING | if state.hotkey_enabled { MF_CHECKED } else { MF_STRING },
        ID_TOGGLE_HOTKEY,
        &hotkey_label(state),
    );

    menu_str(menu, MF_SEPARATOR, 0, "");

    for (i, item) in crate::item::item_definitions().iter().enumerate() {
        let on = state.items.get(i).copied().unwrap_or(false);
        menu_str(
            menu,
            MF_STRING | if on { MF_CHECKED } else { MF_STRING },
            MENU_ITEM_BASE + i as u32,
            &item.label,
        );
    }

    menu_str(menu, MF_SEPARATOR, 0, "");

    menu_str(
        menu,
        MF_STRING | if state.autostart { MF_CHECKED } else { MF_STRING },
        ID_TOGGLE_AUTOSTART,
        "开机自启",
    );
    menu_str(menu, MF_STRING, ID_OPEN_CONFIG, "打开配置文件目录");
    menu_str(menu, MF_SEPARATOR, 0, "");
    menu_str(menu, MF_STRING, ID_QUIT, "退出 WGOpt");
}

fn translate_choice(id: u32) -> Option<MenuChoice> {
    Some(match id {
        0 => return None,
        ID_TOGGLE_ENABLED => MenuChoice::Command(Command::ToggleEnabled),
        ID_TOGGLE_RUN => MenuChoice::Command(Command::ToggleRun),
        ID_TOGGLE_AUTOSTART => MenuChoice::Command(Command::ToggleAutostart),
        ID_TOGGLE_HOTKEY => MenuChoice::Command(Command::ToggleHotkey),
        ID_OPEN_CONFIG => MenuChoice::Command(Command::OpenConfig),
        ID_QUIT => MenuChoice::Command(Command::Quit),
        other if other >= MENU_ITEM_BASE => {
            let idx = (other - MENU_ITEM_BASE) as usize;
            if idx < crate::item::item_definitions().len() {
                MenuChoice::ToggleItem(idx)
            } else {
                return None;
            }
        }
        _ => return None,
    })
}

/// 加载 ico 文件为 HICON（交回系统按 DPI 取用合适尺寸）。
fn load_icon(path: &std::path::Path) -> Option<HICON> {
    if !path.exists() {
        return None;
    }
    let wpath = wide(&path.to_string_lossy());
    unsafe {
        LoadImageW(None, PCWSTR(wpath.as_ptr()), IMAGE_ICON, 0, 0, LR_LOADFROMFILE)
            .ok()
            .map(|h| HICON(h.0))
    }
}

/// 写入固定长度的宽字符缓冲，保证结尾有 NUL。
fn set_text(buf: &mut [u16], text: &str) {
    let max = buf.len().saturating_sub(1);
    let mut n = 0;
    for unit in text.encode_utf16().take(max) {
        buf[n] = unit;
        n += 1;
    }
    buf[n.min(buf.len() - 1)] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(toggle: ToggleAction, hotkey_enabled: bool, hotkey_label: &str) -> MenuState {
        MenuState {
            enabled: !matches!(toggle, ToggleAction::Disabled),
            autostart: false,
            toggle,
            hotkey_enabled,
            hotkey_label: hotkey_label.to_string(),
            status_line: "状态：待机".into(),
            items: vec![true; crate::item::item_definitions().len()],
        }
    }

    /// 快捷键开启时，切换项右侧带按键；关闭时只留菜单文字。
    #[test]
    fn toggle_label_shows_hotkey_only_when_enabled() {
        assert_eq!(
            toggle_label(&state(ToggleAction::Start, true, "Ctrl+F2")),
            "立即开始优化\tCtrl+F2"
        );
        assert_eq!(
            toggle_label(&state(ToggleAction::Start, false, "Ctrl+F2")),
            "立即开始优化"
        );
    }

    /// 已优化时同一个菜单项变成「停止优化并还原」，快捷键提示仍跟着走。
    #[test]
    fn toggle_label_follows_state() {
        assert_eq!(
            toggle_label(&state(ToggleAction::Stop, true, "Ctrl+F2")),
            "停止优化并还原\tCtrl+F2"
        );
        assert_eq!(
            toggle_label(&state(ToggleAction::Busy, true, "Ctrl+F2")),
            "优化进行中…\tCtrl+F2"
        );
        assert_eq!(
            toggle_label(&state(ToggleAction::Disabled, true, "Ctrl+F2")),
            "立即开始优化\tCtrl+F2"
        );
    }

    /// 配置里按键写坏时 `hotkey_label` 为空，不能显示出半截提示。
    #[test]
    fn toggle_label_without_hotkey_text() {
        assert_eq!(toggle_label(&state(ToggleAction::Start, true, "")), "立即开始优化");
    }

    /// 开关关闭时仍展示按键，用户能看到重新打开会绑什么。
    #[test]
    fn hotkey_switch_label_keeps_key_hint() {
        assert_eq!(
            hotkey_label(&state(ToggleAction::Start, true, "Ctrl+F2")),
            "启用快捷键（Ctrl+F2）"
        );
        assert_eq!(
            hotkey_label(&state(ToggleAction::Start, false, "Ctrl+F2")),
            "启用快捷键（Ctrl+F2）"
        );
        assert_eq!(hotkey_label(&state(ToggleAction::Start, true, "")), "启用快捷键");
    }

    /// 只有 Start/Stop 可点；优化中与总开关关闭都灰掉。
    #[test]
    fn menu_item_grayed_when_not_available() {
        assert!(ToggleAction::Start.is_available());
        assert!(ToggleAction::Stop.is_available());
        assert!(!ToggleAction::Busy.is_available());
        assert!(!ToggleAction::Disabled.is_available());
    }

    /// 菜单项 id 决定点击后走哪条命令，改动时最容易漏掉映射。
    #[test]
    fn hotkey_id_maps_to_toggle_command() {
        assert_eq!(
            translate_choice(ID_TOGGLE_HOTKEY),
            Some(MenuChoice::Command(Command::ToggleHotkey))
        );
    }

    /// 切换项只有一个 id：开始与还原不再各占一个。
    #[test]
    fn run_toggle_uses_single_menu_id() {
        assert_eq!(
            translate_choice(ID_TOGGLE_RUN),
            Some(MenuChoice::Command(Command::ToggleRun))
        );
        // 旧的 4（原「停止优化并还原」）不再映射到任何命令
        assert_eq!(translate_choice(4), None);
    }
}
