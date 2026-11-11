//! 手动验证：快捷键在文本编辑场景里不触发。
//!
//! 建一个带 `Edit` 子控件的普通窗口并让它取得前台焦点，然后调用
//! [`wgopt_core::hotkey::editing_reason`]：
//!
//! - 焦点在编辑框里 → 必须返回 `Some`（快捷键应被忽略）
//! - 焦点在窗口本身（没有编辑框）→ 必须返回 `None`（快捷键应正常触发）
//!
//! 运行：
//! ```text
//! cargo run --example hotkey_edit_gate
//! ```
//!
//! 窗口只会闪现一秒左右，不修改任何配置、不执行优化。

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    RegisterClassW, SetForegroundWindow, ShowWindow, TranslateMessage, HMENU, MSG,
    WINDOW_EX_STYLE, WM_DESTROY, WNDCLASSW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use wgopt_core::hotkey::editing_reason;

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_DESTROY {
            // 用 PostQuitMessage 而不是直接退线程，让外层能先读到判定结果
            windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn main() {
    unsafe {
        let instance = GetModuleHandleW(None).expect("模块句柄");
        let class = wide("WGOptHotkeyGateProbe");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let title = wide("WGOpt hotkey gate probe");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            120,
            120,
            420,
            160,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("创建窗口");

        let edit_class = wide("Edit");
        let empty = wide("");
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(edit_class.as_ptr()),
            PCWSTR(empty.as_ptr()),
            WS_CHILD | WS_VISIBLE,
            10,
            10,
            380,
            24,
            Some(hwnd),
            None::<HMENU>,
            Some(instance.into()),
            None,
        )
        .expect("创建 Edit 控件");

        let _ = ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        std::thread::sleep(std::time::Duration::from_millis(250));

        // ---- 1. 焦点在编辑框：应判定为编辑场景 ----
        let _ = SetFocus(Some(edit));
        std::thread::sleep(std::time::Duration::from_millis(150));
        let in_edit = editing_reason();
        println!("[1] 焦点在 Edit 控件：{:?}", in_edit);

        // ---- 2. 焦点在窗口本身：不是编辑场景 ----
        let _ = SetFocus(Some(hwnd));
        std::thread::sleep(std::time::Duration::from_millis(150));
        let in_window = editing_reason();
        println!("[2] 焦点在普通窗口：{:?}", in_window);

        let _ = DestroyWindow(edit);
        let _ = DestroyWindow(hwnd);

        // 收掉可能的挂起消息，避免窗口销毁后消息循环卡住
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        println!();
        if in_edit.is_some() && in_window.is_none() {
            println!("PASS 编辑场景判定正常：编辑框内忽略，普通窗口放行");
        } else {
            println!("FAIL 判定不符合预期（窗口可能没有取得前台焦点，请重试）");
            std::process::exit(1);
        }
    }
}
