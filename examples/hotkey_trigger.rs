//! 手动验证：Ctrl+F2 是**状态切换**——未优化时开始，已优化时还原。
//!
//! 跑的是出货代码路径：`hotkey::trigger` → `Shared::request_toggle` →
//! 后台守护线程执行。重点确认：
//!
//! 1. 待机时按下 → 开始优化（状态变 `Optimized`）
//! 2. 已优化时按下 → 停止并还原（状态回 `Idle`）
//! 3. 还原后再按下 → 又能开始（方向真的切换，不是只会开始）
//! 4. 总开关关闭时按下 → 不触发（`NotAvailable(Disabled)`）
//! 5. 快捷键开关关闭时按下 → 不注册，也不触发（`NotRegistered`）
//! 6. 连按两下 → 第二次被 1 秒闸门或状态拦住
//!
//! 优化项只留 `dns`（刷新解析缓存，无副作用）。
//!
//! 运行：
//! ```text
//! cargo run --release --example hotkey_trigger
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, INPUT_0, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_F2,
};

use wgopt_core::app::{self, Phase, Shared, ToggleAction};
use wgopt_core::config::Config;
use wgopt_core::hotkey::{self, Hotkey, HotkeyBinding, TriggerGate, TriggerOutcome};
use wgopt_core::window;

fn main() {
    let dir = std::env::temp_dir().join("wgopt-hotkey-toggle");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    wgopt_core::logging::init(&dir);
    let log_path = dir.join("wgopt.log");

    // 只跑 dns 项：刷新解析缓存，不需要管理员、不改系统状态
    let mut cfg = Config::default();
    cfg.enabled = true;
    cfg.auto_optimize = false; // 关掉后台检测，避免真游戏干扰
    cfg.enabled_items = vec!["dns".to_string()];
    let shared = Shared::new(cfg);

    let worker = shared.clone();
    std::thread::spawn(move || app::run_guardian(worker));

    let hwnd = window::create_message_window().expect("消息窗口");
    let hk = Hotkey::parse("Ctrl+F2").unwrap();
    let mut binding = HotkeyBinding::new(hwnd);
    let mut gate = TriggerGate::default();
    let mut failed = Vec::new();

    // ---- 1. 待机时按下：应开始 ----
    binding.apply(Some(hk)).unwrap();
    let r = hotkey::trigger(&shared, &binding, &mut gate, Instant::now());
    expect(
        "[1] 待机时按下（应开始）",
        &r,
        TriggerOutcome::Toggled(ToggleAction::Start),
        &mut failed,
    );
    wait_phase(&shared, Phase::Optimized, &mut failed, "[1] 后台未完成优化");

    // ---- 2. 已优化时按下：应还原 ----
    let r = hotkey::trigger(&shared, &binding, &mut gate, Instant::now() + Duration::from_secs(2));
    expect(
        "[2] 已优化时按下（应还原）",
        &r,
        TriggerOutcome::Toggled(ToggleAction::Stop),
        &mut failed,
    );
    wait_phase(&shared, Phase::Idle, &mut failed, "[2] 后台未还原");

    // ---- 3. 还原后再按下：又能开始 ----
    let r = hotkey::trigger(&shared, &binding, &mut gate, Instant::now() + Duration::from_secs(4));
    expect(
        "[3] 还原后再按下（应再次开始）",
        &r,
        TriggerOutcome::Toggled(ToggleAction::Start),
        &mut failed,
    );
    wait_phase(&shared, Phase::Optimized, &mut failed, "[3] 后台未完成第二轮优化");
    // 收尾：还原，避免留下状态
    app::restore_now(&shared, "验证结束");

    // ---- 4. 总开关关闭 ----
    if let Ok(mut c) = shared.config.lock() {
        c.enabled = false;
    }
    shared.sync_enabled();
    let r = hotkey::trigger(&shared, &binding, &mut gate, Instant::now() + Duration::from_secs(6));
    expect(
        "[4] 总开关关闭时按下",
        &r,
        TriggerOutcome::NotAvailable(ToggleAction::Disabled),
        &mut failed,
    );
    if let Ok(mut c) = shared.config.lock() {
        c.enabled = true;
    }
    shared.sync_enabled();

    // ---- 5. 快捷键开关关闭 ----
    if let Ok(mut c) = shared.config.lock() {
        c.hotkey_enabled = false;
    }
    let sync = hotkey::sync_binding(&shared, &mut binding);
    let r = hotkey::trigger(&shared, &binding, &mut gate, Instant::now() + Duration::from_secs(8));
    println!("[5] 关闭快捷键开关后注册结果：{sync:?}，按下结果：{r:?}");
    if !matches!(sync, Ok(None)) {
        failed.push("[5] 关闭开关应注销快捷键".to_string());
    }
    expect(
        "[5] 关闭开关后按下",
        &r,
        TriggerOutcome::NotRegistered,
        &mut failed,
    );

    // ---- 6. 真按键连按两下（走消息循环）----
    if let Ok(mut c) = shared.config.lock() {
        c.hotkey_enabled = true;
    }
    hotkey::sync_binding(&shared, &mut binding).unwrap();
    app::restore_now(&shared, "按键测试前重置");

    let seen = Rc::new(RefCell::new(Vec::<TriggerOutcome>::new()));
    let seen_cb = seen.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(350));
        send_ctrl_f2();
        std::thread::sleep(Duration::from_millis(80));
        send_ctrl_f2(); // 连按：应被闸门或状态拦住
        std::thread::sleep(Duration::from_millis(400));
        window::post_quit();
    });

    // 前面几步用"未来的 Instant"推进过闸门，真实时间追不上，这里换一个干净的
    let binding_cell = Rc::new(RefCell::new(binding));
    let gate_cell = Rc::new(RefCell::new(TriggerGate::default()));
    let shared_cb = shared.clone();
    {
        let binding_cb = binding_cell.clone();
        let gate_cb = gate_cell.clone();
        window::run_message_loop(window::Callbacks {
            tick: Box::new(|| {}),
            show_menu: Box::new(|| {}),
            reattach: Box::new(|| {}),
            hotkey: Box::new(move || {
                let Ok(b) = binding_cb.try_borrow() else {
                    return;
                };
                let Ok(mut g) = gate_cb.try_borrow_mut() else {
                    return;
                };
                let r = hotkey::trigger(&shared_cb, &b, &mut g, Instant::now());
                seen_cb.borrow_mut().push(r);
            }),
        });
    }

    let outcomes = seen.borrow().clone();
    println!("[6] 真按 {hk} 两下，收到 {outcomes:?}");
    match outcomes.first() {
        Some(TriggerOutcome::Toggled(ToggleAction::Start)) => {}
        other => failed.push(format!("[6] 第一下应开始优化，实际 {other:?}")),
    }
    match outcomes.get(1) {
        // 后台可能已经把状态切成 Optimizing/Optimized，那两种拦截也合理
        Some(TriggerOutcome::TooFrequent)
        | Some(TriggerOutcome::NotAvailable(_)) => {}
        Some(other) => failed.push(format!("[6] 连按第二下应被拦住，实际 {other:?}")),
        None => failed.push("[6] 没有收到第二次按下".to_string()),
    }
    wait_phase(&shared, Phase::Optimized, &mut failed, "[6] 真按键未跑完一轮");
    app::restore_now(&shared, "验证结束");

    // ---- 7. 日志：确认两个方向都真执行过（日志由 app 层写，不是 hotkey 层）----
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let optimized = log.matches("开始优化").count();
    let restored = log.matches("停止优化：").count();
    println!("[7] 日志：开始优化 {optimized} 次，停止/还原 {restored} 次");
    if optimized < 2 {
        failed.push(format!("[7] 应至少执行过两轮优化，日志只有 {optimized} 次"));
    }
    if restored < 2 {
        failed.push(format!("[7] 应至少还原过两次，日志只有 {restored} 次"));
    }

    println!();
    if failed.is_empty() {
        println!("PASS Ctrl+F2 按状态切换：待机→开始，已优化→还原，还原后可再开始");
    } else {
        for f in &failed {
            println!("FAIL {f}");
        }
        std::process::exit(1);
    }
}

fn expect(name: &str, got: &TriggerOutcome, want: TriggerOutcome, failed: &mut Vec<String>) {
    println!("{name}：{got:?}");
    if *got != want {
        failed.push(format!("{name} 应为 {want:?}，实际 {got:?}"));
    }
}

/// 等后台线程把状态推到目标值。
fn wait_phase(shared: &Shared, want: Phase, failed: &mut Vec<String>, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while shared.phase() != want && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let got = shared.phase();
    println!("    后台状态：{got:?}（期望 {want:?}）");
    if got != want {
        failed.push(format!("{message}：实际 {got:?}"));
    }
}

/// 模拟一次真实的 Ctrl+F2 按下与抬起。
fn send_ctrl_f2() {
    let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
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
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}
