//! WGOpt 后台常驻程序入口。
//!
//! 启动流程：
//! 1. 单实例检查——已有实例时唤起它的菜单并退出；
//! 2. 初始化日志与配置；
//! 3. 创建不可见消息窗口；
//! 4. 添加托盘图标；
//! 5. 拉起后台检测线程；
//! 6. 进入消息循环，处理托盘菜单。

// 后台常驻程序不弹控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;

use wgopt_core::app::{self, Command, Phase, Shared, Status};
use wgopt_core::config;
use wgopt_core::hotkey::{self, HotkeyBinding};
use wgopt_core::logging;
use wgopt_core::single_instance;
use wgopt_core::tray::{Command as TrayCommand, MenuChoice, MenuState, NotifyKind, Tray, TrayIcon};
use wgopt_core::window;

fn main() {
    // ---- 0. 诊断子命令：不常驻，执行完即退出 ----
    // 必须在提权之前处理：`--version` / `--help` / `--status` 都是只读的，
    // 为了看一眼版本弹 UAC 说不过去；而提权会重启进程，参数虽然透传，
    // 但用户在 UAC 里点「取消」就什么也看不到。
    // 只有真正动系统的子命令才需要管理员，见 [`cli_needs_admin`]。
    let args: Vec<String> = std::env::args().skip(1).collect();
    if cli_needs_admin(&args) {
        wgopt_core::elevate::ensure_admin();
    }
    if let Some(code) = run_cli(&args) {
        std::process::exit(code);
    }

    // 未知参数直接报错退出，而不是当成"没有参数"去常驻：`--autostart`
    // 之外还有历史脚本可能传 `--detect` 之类已移除的开关，静默启动一个
    // 托盘实例反而更难排查。
    if let Some(unknown) = args.iter().find(|a| a.as_str() != "--autostart") {
        attach_console();
        eprintln!("未知的参数：{unknown}");
        eprintln!("用 wgopt --help 查看可用开关。");
        std::process::exit(2);
    }

    // ---- 0.5 常驻模式：与原版一致，运行时 runas 提权 ----
    wgopt_core::elevate::ensure_admin();

    // ---- 1. 单实例 ----
    let Some(_guard) = single_instance::acquire() else {
        // 已有实例在跑：它已经弹出菜单，本进程直接退出
        return;
    };

    // ---- 2. 日志与配置 ----
    logging::init(&config::config_dir());
    logging::info(&format!(
        "WGOpt v{} 启动（PID {}）",
        wgopt_core::VERSION,
        wgopt_core::winapi::current_pid()
    ));

    let cfg = config::load();
    logging::info(&format!(
        "配置：总开关={}，启用项 {} 个",
        cfg.enabled,
        cfg.enabled_items.len()
    ));

    // 首次运行把默认配置落盘：用户按文档去找 config.json 时文件得在那儿，
    // 否则只能靠托盘菜单改，看不到可以手改的字段。
    let cfg_path = config::config_path();
    if !cfg_path.exists() {
        match config::save(&cfg) {
            Ok(()) => logging::info(&format!("已生成默认配置：{}", cfg_path.display())),
            Err(e) => logging::warn(&format!("生成默认配置失败：{e}")),
        }
    }

    let shared = Shared::new(cfg);

    // ---- 3. 消息窗口 ----
    let hwnd = match window::create_message_window() {
        Ok(h) => h,
        Err(e) => {
            logging::error(&format!("创建消息窗口失败：{e}"));
            eprintln!("WGOpt 启动失败：{e}");
            std::process::exit(1);
        }
    };

    // ---- 4. 托盘 ----
    let icon_dir = Tray::resolve_icon_dir();
    logging::info(&format!("托盘图标目录：{}", icon_dir.display()));
    if !icon_dir.join("idle.ico").exists() {
        logging::warn("未找到托盘图标 idle.ico，托盘可能不显示");
    }

    let mut tray = Tray::new(hwnd, icon_dir);
    let initial = shared.status_snapshot();
    if tray.add(icon_for(&initial), &initial.tooltip()) {
        logging::info("托盘图标已注册");
    } else {
        logging::error("Shell_NotifyIcon 添加托盘图标失败，托盘将不可见");
    }

    // 托盘只在主线程访问（tick 与菜单都在消息循环线程上），
    // 用 Rc<RefCell> 即可，不必引入 Mutex——那会在模态菜单期间死锁。
    let tray = Rc::new(RefCell::new(Some(tray)));

    // ---- 4.5 全局快捷键 ----
    // 同样只在主线程访问：注册、菜单里切换开关、WM_HOTKEY 回调都在这一条线程上
    let hotkey_binding = Rc::new(RefCell::new(HotkeyBinding::new(hwnd)));
    let hotkey_gate = Rc::new(RefCell::new(hotkey::TriggerGate::default()));
    match sync_hotkey(&shared, &hotkey_binding) {
        Ok(Some(hk)) => logging::info(&format!("全局快捷键已注册：{hk}")),
        Ok(None) => logging::info("全局快捷键未启用"),
        Err(e) => {
            logging::warn(&format!("全局快捷键未生效：{e}"));
            shared.notify("快捷键未生效", e, NotifyKind::Warn);
        }
    }

    // ---- 5. 后台工作线程 ----
    let worker = shared.clone();
    std::thread::Builder::new()
        .name("wgopt-worker".into())
        .spawn(move || app::run_worker(worker))
        .unwrap_or_else(|e| {
            logging::error(&format!("后台线程启动失败：{e}"));
            std::process::exit(1);
        });

    // 自启场景下补一次"已启动"通知，让用户知道它在跑
    if std::env::args().any(|a| a == "--autostart") {
        logging::info("由开机自启拉起");
    }

    // ---- 6. 菜单处理与消息循环 ----
    let shared_tick = shared.clone();
    let shared_menu = shared.clone();
    let tray_tick = tray.clone();
    let tray_menu = tray.clone();
    let tray_reattach = tray.clone();
    let shared_hotkey = shared.clone();
    let hotkey_menu = hotkey_binding.clone();
    let hotkey_cb = hotkey_binding.clone();
    let gate_cb = hotkey_gate.clone();

    // 第二个实例通过这个命名事件请求弹菜单；每拍检查一次即可，
    // 不必为它再起一个线程或引入 MsgWaitForMultipleObjects。
    let show_event = single_instance::ShowEvent::open();

    window::run_message_loop(window::Callbacks {
        tick: Box::new(move || {
            // 有人再次启动程序：把请求转成消息，走和托盘点击同一条菜单路径
            if show_event.as_ref().map(|e| e.wait(0)).unwrap_or(false) {
                window::post_show_menu();
            }
            // 菜单弹出时 TrackPopupMenu 会跑自己的消息循环，期间 WM_TIMER
            // 仍会进来并占用 tray；此时跳过这一拍，下一拍再刷新。
            let Ok(mut guard) = tray_tick.try_borrow_mut() else {
                return;
            };
            let status = shared_tick.status_snapshot();
            let notifications = shared_tick.drain_notifications();
            if let Some(t) = guard.as_mut() {
                t.update(icon_for(&status), &status.tooltip());
                for n in notifications {
                    t.notify(&n.title, &n.body, n.kind);
                }
            }
        }),
        show_menu: Box::new(move || {
            let state = build_menu_state(&shared_menu, &hotkey_menu);
            let picked = match tray_menu.try_borrow() {
                Ok(guard) => guard.as_ref().and_then(|t| t.popup_menu(&state)),
                Err(_) => None,
            };
            if let Some(choice) = picked {
                handle_choice(&shared_menu, &hotkey_menu, choice);
            }
        }),
        reattach: Box::new(move || {
            if let Ok(mut guard) = tray_reattach.try_borrow_mut() {
                if let Some(t) = guard.as_mut() {
                    t.reattach();
                }
            }
        }),
        hotkey: Box::new(move || {
            let Ok(mut gate) = gate_cb.try_borrow_mut() else {
                return;
            };
            trigger_hotkey(&shared_hotkey, &hotkey_cb, &mut gate);
        }),
    });

    // ---- 收尾 ----
    // 先摘掉托盘：还原里重启 Wallpaper Engine 要等好几秒，期间不能还挂着
    // 一个「已优化」的图标。
    if let Ok(mut guard) = tray.try_borrow_mut() {
        *guard = None;
    }
    logging::info("消息循环结束");
    // 还原只在这里做一次：以快照为准（优化跑到一半退出也要还原），
    // 并且遵守 restore_on_exit。托盘「退出」只负责结束消息循环。
    shared.shutdown();
}



/// 附着到父进程的控制台，让 CLI 输出能被终端看到。
fn attach_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// 命令行开关一览。`run_cli` 与 [`cli_needs_admin`] 共用，避免两边漏掉同一个开关。
const CLI_COMMANDS: &[&str] = &[
    "--status",
    "--optimize-once",
    "--restore",
    "--version",
    "-v",
    "--help",
    "-h",
];

/// 该子命令是否需要管理员权限。
///
/// 只有 `--optimize-once` 真的会停服务、结束进程、改优先级。其余都是
/// 只读查询或纯提示，为它们弹 UAC 只会让排障变麻烦。
fn cli_needs_admin(args: &[String]) -> bool {
    matches!(args.first().map(String::as_str), Some("--optimize-once"))
}

/// 处理命令行诊断子命令。返回 `Some(退出码)` 表示已处理完毕，无需常驻。
///
/// 这些开关不参与后台常驻逻辑，只用于在终端里检查配置与单次优化的
/// 实际效果——后台程序没有界面，排障全靠它们。
fn run_cli(args: &[String]) -> Option<i32> {
    let cmd = args.first()?.as_str();
    if !CLI_COMMANDS.contains(&cmd) {
        return None;
    }

    // release 构建是 windows 子系统，没有自己的控制台；
    // 附着到调用方（终端）的控制台，输出才能被看到。
    attach_console();

    match cmd {
        "--version" | "-v" => {
            println!("WGOpt v{}", wgopt_core::VERSION);
            Some(0)
        }
        "--help" | "-h" => {
            print_help();
            Some(0)
        }
        "--status" => {
            print_status();
            Some(0)
        }
        "--optimize-once" => Some(cli_optimize(false)),
        "--restore" => Some(cli_optimize(true)),
        _ => None,
    }
}

fn print_help() {
    println!("WGOpt v{} · 无界面游戏优化后台程序", wgopt_core::VERSION);
    println!();
    println!("用法：");
    println!("  wgopt                    启动托盘常驻（默认）");
    println!("  wgopt --status           显示配置与运行环境");
    println!("  wgopt --optimize-once    执行一轮优化（不常驻）");
    println!("  wgopt --restore          说明还原入口（快照只在运行中的进程里）");
    println!("  wgopt --version          显示版本");
    println!();
    println!("配置与日志：{}", config::config_dir().display());
}

fn print_status() {
    let cfg = config::load();
    let mem_total = wgopt_core::winapi::total_physical();
    let mem_avail = wgopt_core::winapi::available_physical();

    println!("WGOpt v{}", wgopt_core::VERSION);
    println!("配置目录：{}", config::config_dir().display());
    println!("配置文件：{}", config::config_path().display());
    println!("管理员权限：{}", if wgopt_core::winapi::is_elevated() { "是" } else { "否" });
    println!("物理内存：{} / {}", wgopt_core::winapi::format_bytes(mem_avail), wgopt_core::winapi::format_bytes(mem_total));
    println!();
    println!("总开关：{}", if cfg.enabled { "开" } else { "关" });
    println!("开机自启：{}", if config::is_autostart_enabled() { "开" } else { "关" });
    println!("优化项：");
    for def in wgopt_core::item_definitions() {
        let mark = if cfg.is_item_enabled(&def.id) { "✓" } else { " " };
        println!("  [{mark}] {:<11} {}", def.id, def.label);
    }
}

/// 单次执行优化或还原，输出到控制台。
fn cli_optimize(restore: bool) -> i32 {
    if !wgopt_core::winapi::is_elevated() {
        eprintln!("警告：当前不是管理员权限，部分优化项会失败");
    }
    logging::init(&config::config_dir());

    if restore {
        // 还原需要的快照只存在于运行中的进程里，
        // 独立进程无法感知别的实例做了什么，只能提示。
        println!("快照只保存在运行中的后台进程内。");
        println!("如需还原，请在托盘菜单里选择「停止优化并还原」。");
        return 0;
    }

    let cfg = config::load();
    if !cfg.enabled {
        println!("总开关已关闭，未执行任何优化。");
        return 1;
    }
    if cfg.enabled_items.is_empty() {
        println!("没有启用任何优化项。");
        return 1;
    }

    // 命令行单次优化没有"刚按快捷键"这个上下文，所以没有可识别的前台
    // 进程；用配置里上次记录的目标（若它已被加进排除清单，这里会拒绝）。
    let target = if cfg.game_process.trim().is_empty() {
        None
    } else {
        match wgopt_core::focus::check_target(&cfg, &cfg.game_process) {
            Ok(name) => {
                println!("提升目标（来自配置）：{name}");
                Some(name)
            }
            Err(why) => {
                println!("配置里的 game_process 不可用：{}", why.reason());
                None
            }
        }
    };

    let mut snap = wgopt_core::Snapshot::default();
    println!("开始优化，共 {} 项", cfg.enabled_items.len());
    let results = wgopt_core::engine::run_items(&cfg.enabled_items, target.as_deref(), &mut snap);
    for r in &results {
        let level = match r.status {
            wgopt_core::ItemStatus::Done => "完成",
            wgopt_core::ItemStatus::Skip => "跳过",
            wgopt_core::ItemStatus::Fail => "失败",
        };
        println!("[{level}] {:<11} {:>5}ms  {}", r.id, r.elapsed_ms, r.detail);
    }

    let done = results.iter().filter(|r| r.is_completed()).count();
    let failed = results.iter().filter(|r| r.status == wgopt_core::ItemStatus::Fail).count();
    println!();
    println!("合计：{done}/{} 完成，{failed} 项失败", results.len());
    println!("快照：{}", snap.summary());
    if failed > 0 { 1 } else { 0 }
}

fn icon_for(status: &Status) -> TrayIcon {
    if !status.enabled {
        TrayIcon::Off
    } else if status.phase == Phase::Optimized {
        TrayIcon::Active
    } else {
        TrayIcon::Idle
    }
}

fn build_menu_state(shared: &Shared, binding: &Rc<RefCell<HotkeyBinding>>) -> MenuState {
    let status = shared.status_snapshot();
    let defs = wgopt_core::item_definitions();
    let (items, hotkey_text) = match shared.config.lock() {
        Ok(cfg) => (
            defs.iter().map(|d| cfg.is_item_enabled(&d.id)).collect(),
            cfg.hotkey.clone(),
        ),
        Err(_) => (vec![true; defs.len()], String::new()),
    };

    // 勾选状态以「真的注册上了」为准：组合键被别的程序占用时，配置里
    // 虽然写着 true，但按键并不生效，菜单不能骗人。
    let registered = binding.try_borrow().ok().and_then(|b| b.active());
    // 配置里写坏或为空时不展示按键，菜单里只剩开关本身
    let hotkey_label = registered
        .map(|hk| hk.to_string())
        .or_else(|| hotkey::Hotkey::parse(&hotkey_text).ok().map(|hk| hk.to_string()))
        .unwrap_or_default();

    MenuState {
        enabled: status.enabled,
        autostart: config::is_autostart_enabled(),
        // 菜单文案与灰化来自同一个判定，快捷键也用它
        toggle: shared.toggle_action(),
        hotkey_enabled: registered.is_some(),
        hotkey_label,
        status_line: status.status_line(),
        items,
    }
}

/// 按配置重新注册快捷键，返回生效的组合键（未启用为 `None`）。
fn sync_hotkey(
    shared: &Shared,
    binding: &Rc<RefCell<HotkeyBinding>>,
) -> Result<Option<hotkey::Hotkey>, String> {
    let mut guard = binding
        .try_borrow_mut()
        .map_err(|_| "快捷键状态正忙，请重试".to_string())?;
    hotkey::sync_binding(shared, &mut guard)
}

/// 快捷键触发：按当前状态切换，与点击托盘菜单的切换项走同一个函数。
fn trigger_hotkey(shared: &Shared, binding: &Rc<RefCell<HotkeyBinding>>, gate: &mut hotkey::TriggerGate) {
    let Ok(guard) = binding.try_borrow() else {
        return;
    };
    let outcome = hotkey::trigger(shared, &guard, gate, std::time::Instant::now());
    drop(guard);

    // 失败场景（不可用、间隔太短）也记一行：排障时能看出快捷键被按到了
    logging::info(&outcome.log_line());
}

fn handle_choice(
    shared: &Shared,
    binding: &Rc<RefCell<HotkeyBinding>>,
    choice: MenuChoice,
) {
    match choice {
        MenuChoice::Command(TrayCommand::ToggleEnabled) => toggle_enabled(shared),
        // 菜单点击与快捷键共用同一个状态切换入口，不各自判断方向
        MenuChoice::Command(TrayCommand::ToggleRun) => {
            let (action, command) = shared.request_toggle();
            match command {
                Some(cmd) => logging::info(&format!("菜单执行：{}（{cmd:?}）", action.label())),
                None => logging::info(&format!("菜单点击被忽略：{}", action.label())),
            }
        }
        MenuChoice::Command(TrayCommand::ToggleAutostart) => toggle_autostart(shared),
        MenuChoice::Command(TrayCommand::ToggleHotkey) => toggle_hotkey(shared, binding),
        MenuChoice::Command(TrayCommand::OpenConfig) => open_config_dir(),
        MenuChoice::Command(TrayCommand::Quit) => {
            // 还原与退出统一由 main 的收尾段处理（shared.shutdown），
            // 这里只结束消息循环，避免两个线程各还原一遍。
            window::post_quit();
        }
        MenuChoice::ToggleItem(idx) => toggle_item(shared, idx),
    }
}

fn toggle_enabled(shared: &Shared) {
    let mut now = false;
    app::persist(shared, |cfg| {
        cfg.enabled = !cfg.enabled;
        now = cfg.enabled;
    });
    logging::info(&format!("总开关切换为 {}", now));
    shared.notify(
        "WGOpt",
        if now { "优化已启用" } else { "优化已停用" },
        NotifyKind::Info,
    );

    // 关闭总开关时立刻还原已生效的改动，避免留下副作用。
    // 判定看快照而非 phase：优化跑到一半时 phase 还不是「已优化」，
    // 但优先级、服务、Wallpaper Engine 已经动过了。
    if !now && shared.has_restorable_changes() {
        shared.post_command(Command::Stop);
    }
}

fn toggle_autostart(shared: &Shared) {
    let target = !config::is_autostart_enabled();
    match config::set_autostart(target) {
        Ok(()) => {
            app::persist(shared, |cfg| cfg.autostart = target);
            // 以注册表真实状态为准，而不是假定写入成功
            let real = config::is_autostart_enabled();
            logging::info(&format!("开机自启设置为 {real}"));
            shared.notify(
                "WGOpt",
                if real {
                    "已设置为开机自启"
                } else {
                    "已取消开机自启"
                },
                if real == target {
                    NotifyKind::Info
                } else {
                    NotifyKind::Warn
                },
            );
        }
        Err(e) => {
            logging::error(&format!("设置开机自启失败：{e}"));
            shared.notify("开机自启设置失败", e, NotifyKind::Error);
        }
    }
}

/// 切换快捷键开关：立刻注册/注销，并把结果写回配置。
///
/// 以「现在是否真的注册着」为基准翻转，而不是读配置里的期望值：启动时
/// 组合键可能已被别的程序占用（配置写着 true 但没注册成功），按配置取反
/// 会让用户第一下点击变成"关"，得点两次才能生效。
///
/// 注册失败（组合键被别的程序占用）时回滚配置，避免下次启动仍按"已开启"重试。
fn toggle_hotkey(shared: &Shared, binding: &Rc<RefCell<HotkeyBinding>>) {
    let currently_active = binding
        .try_borrow()
        .map(|b| b.active().is_some())
        .unwrap_or(false);
    let target = !currently_active;

    let mut now = false;
    app::persist(shared, |cfg| {
        cfg.hotkey_enabled = target;
        now = cfg.hotkey_enabled;
    });

    match sync_hotkey(shared, binding) {
        Ok(Some(hk)) => {
            logging::info(&format!("快捷键已开启：{hk}"));
            shared.notify("WGOpt", format!("已启用快捷键 {hk}"), NotifyKind::Info);
        }
        Ok(None) => {
            logging::info("快捷键已关闭");
            shared.notify("WGOpt", "已关闭快捷键", NotifyKind::Info);
        }
        Err(e) => {
            logging::warn(&format!("快捷键开启失败：{e}"));
            if now {
                // 写回关闭状态，下次启动不再尝试注册这个组合键
                app::persist(shared, |cfg| cfg.hotkey_enabled = false);
            }
            shared.notify("快捷键未生效", e, NotifyKind::Warn);
        }
    }
}

fn toggle_item(shared: &Shared, idx: usize) {
    let defs = wgopt_core::item_definitions();
    let Some(def) = defs.get(idx) else {
        return;
    };
    let id = def.id.clone();
    let mut now = false;
    app::persist(shared, |cfg| {
        now = !cfg.is_item_enabled(&id);
        cfg.set_item_enabled(&id, now);
    });
    logging::info(&format!("优化项 {id} 切换为 {}", now));
}

/// 用资源管理器打开配置文件所在目录。
fn open_config_dir() {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let dir = config::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };

    let op = wide("open");
    let path = wide(&dir.to_string_lossy());
    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(op.as_ptr()),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}
