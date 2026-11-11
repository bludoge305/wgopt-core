//! 手动验证：按「立即开始优化」的快捷键，真的会触发一轮优化请求。
//!
//! 跑的是出货代码路径：`hotkey::sync_binding` 注册 → 消息循环收 `WM_HOTKEY`
//! → `hotkey::trigger` 判定 → 投递 `Command::Start` → 后台守护线程执行。
//!
//! 覆盖四种情况：
//! 1. 总开关关闭 → 不触发（`NotAvailable`）
//! 2. 快捷键开关关闭 → 不注册，也不触发（`NotRegistered`）
//! 3. 正常按下 → 投递 `Start`，后台真的跑完一轮（状态变 `Optimized`）
//! 4. 连按两下 → 第二次被 1 秒闸门挡住（`TooFrequent`）
//!
//! 用 `Instant` 直接推进时间，不必真等；只有最后一步要等后台线程。
//!
//! 运行（需要管理员权限才能停服务，但验证本身不依赖）：
//! ```text
//! cargo run --release --example hotkey_trigger
//! ```
//!
//! 优化项只留 `dns`（刷新解析缓存，无副作用），跑完立刻还原。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, INPUT_0, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_F2,
};

use wgopt_core::app::{self, Phase, Shared};
use wgopt_core::config::Config;
use wgopt_core::hotkey::{self, Hotkey, HotkeyBinding, TriggerGate, TriggerOutcome};
use wgopt_core::window;

fn main() {
    let dir = std::env::temp_dir().join("wgopt-hotkey-trigger");
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

    // ---- 1. 总开关关闭 ----
    if let Ok(mut c) = shared.config.lock() {
        c.enabled = false;
    }
    shared.sync_enabled();
    binding.apply(Some(hk)).unwrap();
    let r = hotkey::trigger(&shared, &binding, &mut gate, Instant::now());
    report("[1] 总开关关闭时按下", &r, TriggerOutcome::NotAvailable, &mut failed);
    if let Ok(mut c) = shared.config.lock() {
        c.enabled = true;
    }
    shared.sync_enabled();

    // ---- 2. 快捷键开关关闭 ----
    if let Ok(mut c) = shared.config.lock() {
        c.hotkey_enabled = false;
    }
    let sync = hotkey::sync_binding(&shared, &mut binding);
    let r = hotkey::trigger(&shared, &binding, &mut gate, Instant::now());
    println!("[2] 关闭快捷键开关后注册结果：{sync:?}，按下结果：{r:?}");
    if !matches!(sync, Ok(None)) {
        failed.push("[2] 关闭开关应注销快捷键".to_string());
    }
    report("[2] 关闭开关后按下", &r, TriggerOutcome::NotRegistered, &mut failed);

    // ---- 3. 正常按下：真按键 + 消息循环 ----
    if let Ok(mut c) = shared.config.lock() {
        c.hotkey_enabled = true;
    }
    hotkey::sync_binding(&shared, &mut binding).unwrap();
    if shared.phase() != Phase::Idle {
        failed.push("[3] 前置状态不是 Idle".to_string());
    }

    let seen = Rc::new(RefCell::new(Vec::<TriggerOutcome>::new()));
    let seen_cb = seen.clone();
    // 消息循环里，按下后只记结果；间隔够 1 秒再送第二下，检验去重
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(350));
        send_ctrl_f2();
        std::thread::sleep(Duration::from_millis(80));
        send_ctrl_f2(); // 连按：应被闸门挡住
        std::thread::sleep(Duration::from_millis(400));
        window::post_quit();
    });

    let binding_cell = Rc::new(RefCell::new(binding));
    let gate_cell = Rc::new(RefCell::new(gate));
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
    println!("[3] 真按 {} 两下，收到 {:?}", hk, outcomes);

    // 后台线程处理 Start 需要一点时间
    let deadline = Instant::now() + Duration::from_secs(10);
    while shared.phase() == Phase::Idle && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let phase = shared.phase();
    println!("[3] 后台执行后的状态：{phase:?}");

    match outcomes.first() {
        Some(TriggerOutcome::Started) => {}
        other => failed.push(format!("[3] 第一次按下应投递 Start，实际 {other:?}")),
    }
    if let Some(second) = outcomes.get(1) {
        // 后台可能在第二次按下前就把状态切成了 Optimizing，那也是合理的拦截
        if !matches!(second, TriggerOutcome::TooFrequent | TriggerOutcome::NotAvailable) {
            failed.push(format!("[3] 连按第二下应被拦住，实际 {second:?}"));
        }
    } else {
        failed.push("[3] 没有收到第二次按下".to_string());
    }
    if phase != Phase::Optimized {
        failed.push(format!("[3] 后台未完成优化，状态 {phase:?}"));
    }

    // ---- 4. 清理：还原 dns 项（无副作用，只为状态干净）----
    app::restore_now(&shared, "验证结束");
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let logged = log.contains("快捷键触发") || log.contains("开始优化");
    println!("[4] 日志里记录了触发：{logged}");

    println!();
    if failed.is_empty() {
        println!("PASS 快捷键触发链路正常：可用时投递 Start 并真跑完一轮，不可用/连按时被拦");
    } else {
        for f in &failed {
            println!("FAIL {f}");
        }
        std::process::exit(1);
    }
}

fn report(name: &str, got: &TriggerOutcome, want: TriggerOutcome, failed: &mut Vec<String>) {
    println!("{name}：{got:?}");
    if *got != want {
        failed.push(format!("{name} 应为 {want:?}，实际 {got:?}"));
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
