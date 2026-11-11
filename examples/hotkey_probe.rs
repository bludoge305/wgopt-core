//! 手动验证：全局快捷键的注册、去重与触发链路。
//!
//! 检查四件事：
//! 1. 配置默认值 `Ctrl+F2` 能注册成功；
//! 2. 同一组合键再注册一次会被系统拒绝（说明它确实占住了全局）；
//! 3. 用 `SendInput` 真按一次，消息窗口能收到 `WM_HOTKEY` 并触发回调；
//! 4. 注销后同一个组合键可以被别的窗口注册。
//!
//! 运行：
//! ```text
//! cargo run --example hotkey_probe
//! ```
//!
//! 不需要管理员权限，也不改配置、不做任何优化。

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_CONTROL, VK_F2,
};
use wgopt_core::hotkey::{self, HotkeyBinding};
use wgopt_core::window;

fn main() {
    let hwnd = match window::create_message_window() {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL 创建消息窗口失败：{e}");
            std::process::exit(1);
        }
    };

    let hk = hotkey::Hotkey::parse(hotkey::DEFAULT_HOTKEY).expect("默认快捷键应能解析");
    let mut binding = HotkeyBinding::new(hwnd);

    // ---- 1. 注册 ----
    match binding.apply(Some(hk)) {
        Ok(()) => println!("[1] 注册 {hk}：成功"),
        Err(e) => {
            println!("[1] 注册 {hk}：失败 —— {e}（是不是有别的程序占用了？）");
            std::process::exit(1);
        }
    }

    // ---- 2. 重复注册应被系统拒绝 ----
    let dup = try_register_another(hwnd, &hk);
    println!("[2] 已注册后，再用另一个 id 注册：{dup}");

    // ---- 3. 真按一次 ----
    let hits = Rc::new(Cell::new(0u32));
    let hits_cb = hits.clone();
    let quit_at = Rc::new(Cell::new(None::<Instant>));

    let sender = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(400));
        send_ctrl_f2();
        std::thread::sleep(Duration::from_millis(600));
        // 收工：让消息循环退出
        window::post_quit();
    });

    let before = Instant::now();
    window::run_message_loop(window::Callbacks {
        tick: Box::new(|| {}),
        show_menu: Box::new(|| {}),
        reattach: Box::new(|| {}),
        hotkey: Box::new(move || hits_cb.set(hits_cb.get() + 1)),
    });
    let elapsed = before.elapsed();
    let _ = sender.join();
    let _ = quit_at;

    let count = hits.get();
    println!("[3] 按下 {}：收到 {count} 次 WM_HOTKEY（耗时 {} ms）", hk, elapsed.as_millis());

    // ---- 4. 注销后应能被别人抢到 ----
    drop(binding);
    let after = try_register_another(hwnd, &hk);
    println!("[4] 注销后再注册同一个组合键：{after}");

    println!();
    if count == 1 && dup.starts_with("被拒绝") && after == "成功" {
        println!("PASS 快捷键注册/触发/注销链路正常");
    } else {
        println!("FAIL 结果不符合预期");
        std::process::exit(1);
    }
}

/// 用另一个 id 再注册一次，验证全局唯一性。
/// 直接调 Win32：这里要故意复现失败，不能走 `HotkeyBinding` 的幂等短路。
fn try_register_another(hwnd: windows::Win32::Foundation::HWND, hk: &hotkey::Hotkey) -> String {
    match hotkey::raw_register(hwnd, 99, *hk) {
        Ok(()) => {
            hotkey::raw_unregister(hwnd, 99);
            "成功".to_string()
        }
        Err(e) => format!("被拒绝（{e}）"),
    }
}

/// 模拟一次真实的 Ctrl+F2 按下与抬起。
fn send_ctrl_f2() {
    let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };

    let inputs = [
        key(VK_CONTROL, false),
        key(VK_F2, false),
        key(VK_F2, true),
        key(VK_CONTROL, true),
    ];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        println!("     （SendInput 只送进了 {sent}/{} 个事件）", inputs.len());
    }
}
