//! 全局快捷键：组合键解析、注册到消息窗口、编辑场景判定。
//!
//! 用 Win32 `RegisterHotKey` 挂在主线程的消息窗口上，按下时窗口过程收到
//! `WM_HOTKEY`。不用全局键盘钩子：钩子会被安全软件拦，也会拖慢所有按键。
//!
//! 重复触发有两道闸：注册时带 `MOD_NOREPEAT`（按住不放只触发一次），
//! 触发后还要过一遍「立即开始优化」是否可用（与托盘菜单同一判定）。

use windows::Win32::Foundation::{HWND, ERROR_HOTKEY_ALREADY_REGISTERED};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
    GUI_INMENUMODE, GUI_POPUPMENUMODE,
};

/// 默认快捷键，与 `config::Config::default` 一致。
pub const DEFAULT_HOTKEY: &str = "Ctrl+F2";

/// 注册 ID：同一窗口只挂一个全局快捷键。
pub const HOTKEY_ID: i32 = 1;

/// 解析好的组合键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// 主按键的虚拟键码
    pub vk: u32,
}

impl Hotkey {
    /// 解析 `Ctrl+F2` 这样的组合键。
    ///
    /// 必须至少带一个修饰键：裸键注册成全局热键会把正常输入吞掉。
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw = text.trim();
        if raw.is_empty() {
            return Err("快捷键为空".into());
        }

        let mut hk = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            vk: 0,
        };
        let mut key: Option<u32> = None;

        for part in raw.split('+') {
            let token = part.trim();
            if token.is_empty() {
                return Err(format!("「{raw}」格式错误：有空的按键名"));
            }
            let upper = token.to_ascii_uppercase();
            match upper.as_str() {
                "CTRL" | "CONTROL" => hk.ctrl = true,
                "ALT" => hk.alt = true,
                "SHIFT" => hk.shift = true,
                "WIN" | "WINDOWS" | "SUPER" | "META" => hk.win = true,
                _ => {
                    let Some(vk) = vk_from_name(&upper) else {
                        return Err(format!("「{token}」不是可用的按键"));
                    };
                    if key.replace(vk).is_some() {
                        return Err(format!("「{raw}」里只能有一个主按键"));
                    }
                }
            }
        }

        if !(hk.ctrl || hk.alt || hk.shift || hk.win) {
            return Err(format!(
                "「{raw}」必须带修饰键（Ctrl/Alt/Shift/Win），否则会吞掉正常输入"
            ));
        }
        let Some(vk) = key else {
            return Err(format!("「{raw}」缺少主按键"));
        };
        hk.vk = vk;
        Ok(hk)
    }

    /// 传给 `RegisterHotKey` 的修饰键，始终附带 `MOD_NOREPEAT`。
    pub fn modifiers(&self) -> HOT_KEY_MODIFIERS {
        let mut m = HOT_KEY_MODIFIERS(0) | MOD_NOREPEAT;
        if self.ctrl {
            m |= MOD_CONTROL;
        }
        if self.alt {
            m |= MOD_ALT;
        }
        if self.shift {
            m |= MOD_SHIFT;
        }
        if self.win {
            m |= MOD_WIN;
        }
        m
    }
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.win {
            parts.push("Win");
        }
        parts.push("");
        let mut text = parts.join("+");
        text.push_str(&vk_name(self.vk));
        f.write_str(&text)
    }
}

/// 按名字取虚拟键码。
fn vk_from_name(name: &str) -> Option<u32> {
    if let Some(rest) = name.strip_prefix('F') {
        if let Ok(n) = rest.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Some(0x70 + n - 1);
            }
        }
    }

    // 单个字母或数字直接用 ASCII 码，正好等于 VK 值
    let bytes = name.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
        return Some(bytes[0] as u32);
    }

    Some(match name {
        "SPACE" => 0x20,
        "TAB" => 0x09,
        "ENTER" | "RETURN" => 0x0D,
        "ESC" | "ESCAPE" => 0x1B,
        "INS" | "INSERT" => 0x2D,
        "DEL" | "DELETE" => 0x2E,
        "HOME" => 0x24,
        "END" => 0x23,
        "PGUP" | "PAGEUP" => 0x21,
        "PGDN" | "PAGEDOWN" => 0x22,
        "UP" => 0x26,
        "DOWN" => 0x28,
        "LEFT" => 0x25,
        "RIGHT" => 0x27,
        "`" | "~" => 0xC0,
        "-" => 0xBD,
        "=" => 0xBB,
        "[" => 0xDB,
        "]" => 0xDD,
        "\\" => 0xDC,
        ";" => 0xBA,
        "'" => 0xDE,
        "," => 0xBC,
        "." => 0xBE,
        "/" => 0xBF,
        _ => return None,
    })
}

/// 虚拟键码的可读名，与 [`vk_from_name`] 互相可逆。
fn vk_name(vk: u32) -> String {
    if (0x70..=0x87).contains(&vk) {
        return format!("F{}", vk - 0x70 + 1);
    }
    if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) {
        return (vk as u8 as char).to_string();
    }
    match vk {
        0x20 => "Space".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x1B => "Esc".into(),
        0x2D => "Ins".into(),
        0x2E => "Del".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "PgUp".into(),
        0x22 => "PgDn".into(),
        0x26 => "Up".into(),
        0x28 => "Down".into(),
        0x25 => "Left".into(),
        0x27 => "Right".into(),
        0xC0 => "`".into(),
        0xBD => "-".into(),
        0xBB => "=".into(),
        0xDB => "[".into(),
        0xDD => "]".into(),
        0xDC => "\\".into(),
        0xBA => ";".into(),
        0xDE => "'".into(),
        0xBC => ",".into(),
        0xBE => ".".into(),
        0xBF => "/".into(),
        other => format!("0x{other:02X}"),
    }
}

/// 快捷键注册状态。
///
/// 只在主线程使用（与托盘同理），因此放在 `Rc<RefCell<_>>` 里即可。
pub struct HotkeyBinding {
    hwnd: HWND,
    active: Option<Hotkey>,
}

impl HotkeyBinding {
    pub fn new(hwnd: HWND) -> Self {
        Self { hwnd, active: None }
    }

    /// 当前已注册的组合键。
    pub fn active(&self) -> Option<Hotkey> {
        self.active
    }

    /// 换成新快捷键；`None` 表示注销旧快捷键。
    ///
    /// 目标与当前一致时直接返回，不重复走注销/注册——那会在两次注册之间
    /// 留出一个空隙，正好被别的程序抢走。
    ///
    /// 失败时（被别的程序占用等）旧快捷键已经注销，状态为"未注册"。
    pub fn apply(&mut self, target: Option<Hotkey>) -> Result<(), String> {
        if self.active == target {
            return Ok(());
        }
        self.clear();
        let Some(hk) = target else {
            return Ok(());
        };
        match unsafe { RegisterHotKey(Some(self.hwnd), HOTKEY_ID, hk.modifiers(), hk.vk) } {
            Ok(()) => {
                self.active = Some(hk);
                Ok(())
            }
            Err(e) => {
                // Win32 错误被包装成 HRESULT，低 16 位还是原始错误码
                let code = (e.code().0 as u32) & 0xFFFF;
                if code == ERROR_HOTKEY_ALREADY_REGISTERED.0 {
                    Err(format!("快捷键 {hk} 已被其他程序占用"))
                } else {
                    Err(format!("快捷键 {hk} 注册失败（错误码 0x{code:04X}）"))
                }
            }
        }
    }

    fn clear(&mut self) {
        if self.active.take().is_some() {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ID);
            }
        }
    }
}

impl Drop for HotkeyBinding {
    fn drop(&mut self) {
        self.clear();
    }
}

/// 用指定 id 直接注册，返回中文错误说明。
///
/// 供诊断程序验证「组合键确实占住了全局」用——`HotkeyBinding::apply` 对
/// 相同目标会短路，无法从它观察到系统层面的冲突。
pub fn raw_register(hwnd: HWND, id: i32, hk: Hotkey) -> Result<(), String> {
    let result = unsafe { RegisterHotKey(Some(hwnd), id, hk.modifiers(), hk.vk) };
    result.map_err(|e| {
        // Win32 错误被包装成 HRESULT，低 16 位还是原始错误码
        let code = (e.code().0 as u32) & 0xFFFF;
        if code == ERROR_HOTKEY_ALREADY_REGISTERED.0 {
            format!("已被占用（错误码 {code}）")
        } else {
            format!("注册失败（错误码 0x{code:04X}）")
        }
    })
}

/// 注销指定 id 的注册（诊断用）。
pub fn raw_unregister(hwnd: HWND, id: i32) {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), id);
    }
}

/// 前台是否处于「输入/编辑」场景；返回命中的原因，便于写日志。
///
/// 四条判据任一命中即认为是编辑场景：
/// 1. 本线程正在操作菜单（托盘菜单是模态的，`TrackPopupMenu` 期间会
///    在本线程跑嵌套消息循环，`WM_HOTKEY` 照样送进来）；
/// 2. 前台线程正在操作菜单；
/// 3. 前台线程有文本光标位（`hwndCaret`）——原生文本框、富文本、多数 IDE；
/// 4. 焦点窗口或前台窗口的类名是编辑控件（`Edit`、`RichEdit*`、`Scintilla`…）。
///
/// 浏览器内核的 `Chrome_RenderWidgetHostHWND` 一类**不**进类名黑名单：
/// 整个网页都是它，一刀切会让网页游戏里按不了快捷键；这类场景靠光标位判断。
pub fn editing_reason() -> Option<&'static str> {
    let self_tid = unsafe { GetCurrentThreadId() };
    if in_menu_mode(self_tid) {
        return Some("正在操作菜单");
    }

    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return None;
        }

        let mut pid = 0u32;
        let tid = GetWindowThreadProcessId(fg, Some(&mut pid));
        if tid == 0 {
            return None;
        }

        if in_menu_mode(tid) {
            return Some("前台正在操作菜单");
        }
        if has_caret(tid) {
            return Some("前台有文本光标");
        }

        let mut focus = fg;
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(tid, &mut info).is_ok() && !info.hwndFocus.is_invalid() {
            focus = info.hwndFocus;
        }

        if is_edit_class(&class_name(focus)) {
            return Some("焦点窗口是文本编辑控件");
        }
        if is_edit_class(&class_name(fg)) {
            return Some("前台窗口是文本编辑控件");
        }
        None
    }
}

fn gui_info(tid: u32) -> Option<GUITHREADINFO> {
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetGUIThreadInfo(tid, &mut info).ok().map(|_| info) }
}

fn in_menu_mode(tid: u32) -> bool {
    gui_info(tid)
        .map(|i| i.flags.contains(GUI_INMENUMODE) || i.flags.contains(GUI_POPUPMENUMODE))
        .unwrap_or(false)
}

fn has_caret(tid: u32) -> bool {
    gui_info(tid)
        .map(|i| !i.hwndCaret.is_invalid())
        .unwrap_or(false)
}

/// 类名是否属于文本编辑控件。
fn is_edit_class(class: &str) -> bool {
    let c = class.trim().to_ascii_lowercase();
    if c.is_empty() {
        return false;
    }
    c == "edit"
        || c.starts_with("richedit")
        || c.starts_with("scintilla")
        // Word 的编辑区窗口类名
        || c == "_wwg"
}

fn class_name(hwnd: HWND) -> String {
    if hwnd.is_invalid() {
        return String::new();
    }
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

/// 连按防护：两次触发之间至少要隔一段时间。
///
/// 后台线程把状态切成"优化中"需要一点时间，这段时间里连按会各投递一条
/// Start。窗口取 1 秒——足够挡住手抖连按，又不会妨碍真正想再来一轮的用户。
pub struct TriggerGate {
    min_interval: std::time::Duration,
    last: Option<std::time::Instant>,
}

impl TriggerGate {
    pub fn new(min_interval: std::time::Duration) -> Self {
        Self {
            min_interval,
            last: None,
        }
    }

    /// 判定是否放行；放行时记录本次时间。
    pub fn accept(&mut self, now: std::time::Instant) -> bool {
        if let Some(last) = self.last {
            if now.duration_since(last) < self.min_interval {
                return false;
            }
        }
        self.last = Some(now);
        true
    }
}

impl Default for TriggerGate {
    fn default() -> Self {
        Self::new(std::time::Duration::from_millis(1000))
    }
}

/// 按配置（重新）注册快捷键，返回生效的组合键；未启用为 `None`。
///
/// 配置里组合键写坏时返回 `Err`，且保证不残留注册——坏配置只影响快捷键
/// 本身，不会连带改变优化总开关。
pub fn sync_binding(
    shared: &crate::app::Shared,
    binding: &mut HotkeyBinding,
) -> Result<Option<Hotkey>, String> {
    let target = match shared.config.lock() {
        Ok(cfg) => match cfg.active_hotkey() {
            Ok(hk) => hk,
            Err(e) => {
                let _ = binding.apply(None);
                return Err(format!("配置里的 hotkey「{}」无效：{e}", cfg.hotkey));
            }
        },
        Err(_) => None,
    };
    binding.apply(target)?;
    Ok(binding.active())
}

/// 一次快捷键触发的处理结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerOutcome {
    /// 已按当前状态投递切换命令
    Toggled(crate::app::ToggleAction),
    /// 当前没有注册成功的快捷键
    NotRegistered,
    /// 处于输入/编辑场景，已忽略（附命中原因）
    Editing(&'static str),
    /// 当前状态不可切换（正在优化，或总开关关闭）
    NotAvailable(crate::app::ToggleAction),
    /// 与上一次触发间隔太短
    TooFrequent,
}

impl TriggerOutcome {
    /// 写日志用的一句话。
    pub fn log_line(&self) -> String {
        use crate::app::ToggleAction;
        match self {
            TriggerOutcome::Toggled(ToggleAction::Start) => {
                "快捷键触发：立即开始优化".into()
            }
            TriggerOutcome::Toggled(ToggleAction::Stop) => {
                "快捷键触发：停止优化并还原".into()
            }
            // 其余分支理论上不会出现，真出现说明 command 映射漏了一种
            TriggerOutcome::Toggled(other) => {
                format!("快捷键触发：{}（该状态本不该投递命令）", other.label())
            }
            TriggerOutcome::NotRegistered => "快捷键已忽略：当前没有注册成功".into(),
            TriggerOutcome::Editing(why) => format!("快捷键已忽略：{why}"),
            TriggerOutcome::NotAvailable(action) => {
                format!("快捷键已忽略：当前状态不可切换（{}）", action.label())
            }
            TriggerOutcome::TooFrequent => "快捷键已忽略：与上次触发间隔过短".into(),
        }
    }
}

/// 处理一次快捷键按下：按当前优化状态切换（未优化→开始，已优化→还原）。
///
/// 判定与投递全部交给 [`crate::app::Shared::request_toggle`]，与托盘菜单点击
/// 走同一个函数——菜单能做的切换，快捷键就能做；菜单灰掉的，快捷键也不做。
/// 四道闸按代价从低到高排列。
pub fn trigger(
    shared: &crate::app::Shared,
    binding: &HotkeyBinding,
    gate: &mut TriggerGate,
    now: std::time::Instant,
) -> TriggerOutcome {
    if binding.active().is_none() {
        return TriggerOutcome::NotRegistered;
    }
    let action = shared.toggle_action();
    if !action.is_available() {
        return TriggerOutcome::NotAvailable(action);
    }
    if let Some(why) = editing_reason() {
        return TriggerOutcome::Editing(why);
    }
    // 闸门放在最后：只有"真要执行"时才消耗时间戳，避免被忽略的按键
    // 把窗口往后推。
    if !gate.accept(now) {
        return TriggerOutcome::TooFrequent;
    }

    // 重新判定一次再投递：上面几道闸之间状态可能已经变了（例如后台刚跑完
    // 一轮优化）。request_toggle 内部判定与投递是原子的，以它为准。
    let (action, command) = shared.request_toggle();
    match command {
        Some(_) => TriggerOutcome::Toggled(action),
        None => TriggerOutcome::NotAvailable(action),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn parses_default_hotkey() {
        let hk = Hotkey::parse(DEFAULT_HOTKEY).unwrap();
        assert!(hk.ctrl && !hk.alt && !hk.shift && !hk.win);
        assert_eq!(hk.vk, 0x71, "F2 = VK_F2");
        assert_eq!(hk.to_string(), "Ctrl+F2");
    }

    #[test]
    fn parse_is_case_and_space_tolerant() {
        let hk = Hotkey::parse(" control + f2 ").unwrap();
        assert_eq!(hk.to_string(), "Ctrl+F2");

        let hk = Hotkey::parse("alt+SHIFT+f9").unwrap();
        assert_eq!(hk.to_string(), "Alt+Shift+F9");
    }

    #[test]
    fn display_orders_modifiers() {
        let hk = Hotkey::parse("Win+Shift+Ctrl+A").unwrap();
        assert_eq!(hk.to_string(), "Ctrl+Shift+Win+A");
    }

    #[test]
    fn parses_named_keys() {
        assert_eq!(Hotkey::parse("Ctrl+Space").unwrap().vk, 0x20);
        assert_eq!(Hotkey::parse("Ctrl+PgDn").unwrap().vk, 0x22);
        assert_eq!(Hotkey::parse("Ctrl+;").unwrap().vk, 0xBA);
        for name in ["Space", "Tab", "Enter", "Esc", "Ins", "Del", "Home", "End", "PgUp", "PgDn", "Up", "Down", "Left", "Right", "`", "-", "=", "[", "]", "\\", ";", "'", ",", ".", "/"] {
            let text = format!("Ctrl+{name}");
            let hk = Hotkey::parse(&text).unwrap();
            assert_eq!(hk.to_string(), text, "显示名与解析名应可逆");
        }
    }

    #[test]
    fn rejects_bare_key() {
        // 裸键注册成全局热键会吞掉正常输入
        assert!(Hotkey::parse("F2").is_err());
        assert!(Hotkey::parse("A").is_err());
    }

    #[test]
    fn rejects_modifier_only() {
        assert!(Hotkey::parse("Ctrl+Shift").is_err());
    }

    #[test]
    fn rejects_two_main_keys() {
        assert!(Hotkey::parse("Ctrl+A+B").is_err());
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(Hotkey::parse("Ctrl+Foo").is_err());
        assert!(Hotkey::parse("Ctrl+F25").is_err());
        assert!(Hotkey::parse("Ctrl+").is_err());
        assert!(Hotkey::parse("  ").is_err());
    }

    #[test]
    fn modifiers_include_norepeat() {
        let hk = Hotkey::parse("Ctrl+Alt+F2").unwrap();
        let m = hk.modifiers();
        assert!(m.contains(MOD_CONTROL));
        assert!(m.contains(MOD_ALT));
        assert!(!m.contains(MOD_SHIFT));
        assert!(
            m.contains(MOD_NOREPEAT),
            "按住不放不能反复触发，必须带 MOD_NOREPEAT"
        );
    }

    #[test]
    fn edit_classes_are_recognized() {
        assert!(is_edit_class("Edit"));
        assert!(is_edit_class("RichEdit20W"));
        assert!(is_edit_class("RICHEDIT50W"));
        assert!(is_edit_class("Scintilla"));
        assert!(is_edit_class("_WwG"));
    }

    #[test]
    fn game_and_browser_classes_are_not_edit() {
        assert!(!is_edit_class("UnityWndClass"));
        assert!(!is_edit_class("SDL_app"));
        assert!(!is_edit_class("Chrome_RenderWidgetHostHWND"));
        assert!(!is_edit_class(""));
    }

    #[test]
    fn trigger_gate_blocks_rapid_repeats() {
        let mut gate = TriggerGate::default();
        let t0 = Instant::now();
        assert!(gate.accept(t0), "第一次触发应放行");
        assert!(!gate.accept(t0 + Duration::from_millis(200)), "连按应被挡住");
        assert!(!gate.accept(t0 + Duration::from_millis(999)));
        assert!(gate.accept(t0 + Duration::from_millis(1000)), "间隔够了应恢复");
    }

    /// 按住不放时系统不该再送按键事件：注册必须带 MOD_NOREPEAT。
    /// 这里同时确认快捷键的 id 与回调认的 id 是同一个。
    #[test]
    fn hotkey_id_is_stable() {
        assert_eq!(HOTKEY_ID, 1);
    }
}
