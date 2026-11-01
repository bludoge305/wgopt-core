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
use wgopt_core::logging;
use wgopt_core::single_instance;
use wgopt_core::tray::{Command as TrayCommand, MenuChoice, MenuState, NotifyKind, Tray, TrayIcon};
use wgopt_core::window;

fn main() {
    // ---- 0. 管理员权限（与原版一致，运行时 runas 提权）----
    wgopt_core::elevate::ensure_admin();

    // ---- 0.5 诊断子命令：不常驻，执行完即退出 ----
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = run_cli(&args) {
        std::process::exit(code);
    }

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
        "配置：总开关={}，自动优化={}，启用项 {} 个",
        cfg.enabled,
        cfg.auto_optimize,
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

    // ---- 5. 后台检测线程 ----
    let worker = shared.clone();
    std::thread::Builder::new()
        .name("wgopt-guardian".into())
        .spawn(move || app::run_guardian(worker))
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

    window::run_message_loop(window::Callbacks {
        tick: Box::new(move || {
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
            let state = build_menu_state(&shared_menu);
            let picked = match tray_menu.try_borrow() {
                Ok(guard) => guard.as_ref().and_then(|t| t.popup_menu(&state)),
                Err(_) => None,
            };
            if let Some(choice) = picked {
                handle_choice(&shared_menu, choice);
            }
        }),
        reattach: Box::new(move || {
            if let Ok(mut guard) = tray_reattach.try_borrow_mut() {
                if let Some(t) = guard.as_mut() {
                    t.reattach();
                }
            }
        }),
    });

    // ---- 收尾 ----
    logging::info("消息循环结束");
    if shared.phase() == Phase::Optimized && shared.config.lock().map(|c| c.restore_on_exit).unwrap_or(true) {
        app::restore_now(&shared, "程序退出，已自动恢复");
    }
    if let Ok(mut guard) = tray.try_borrow_mut() {
        *guard = None;
    }
    logging::info("WGOpt 已退出");
}



/// 附着到父进程的控制台，让 CLI 输出能被终端看到。
fn attach_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// 处理命令行诊断子命令。返回 `Some(退出码)` 表示已处理完毕，无需常驻。
///
/// 这些开关不参与后台常驻逻辑，只用于在终端里检查配置、识别结果与
/// 单次优化的实际效果——后台程序没有界面，排障全靠它们。
fn run_cli(args: &[String]) -> Option<i32> {
    let cmd = args.first()?.as_str();
    if !matches!(
        cmd,
        "--status" | "--detect" | "--optimize-once" | "--restore" | "--version" | "--help" | "-h"
    ) {
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
        "--detect" => {
            let hit = detect_once();
            match hit {
                Some(g) => {
                    println!("检测到游戏：{}（{} · PID {}）", g.label(), g.name, g.pid);
                    println!("判定得分：{}", g.score);
                    for r in &g.reasons {
                        println!("  · {r}");
                    }
                    Some(0)
                }
                None => {
                    println!("未检测到游戏");
                    Some(1)
                }
            }
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
    println!("  wgopt --detect           执行一轮游戏识别并输出判定依据");
    println!("  wgopt --optimize-once    执行一轮优化（不常驻）");
    println!("  wgopt --restore          还原优先级与服务（不常驻）");
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
    println!("自动优化：{}", if cfg.auto_optimize { "开" } else { "关" });
    println!("开机自启：{}", if config::is_autostart_enabled() { "开" } else { "关" });
    println!("优化项：");
    for def in wgopt_core::item_definitions() {
        let mark = if cfg.is_item_enabled(&def.id) { "✓" } else { " " };
        println!("  [{mark}] {:<11} {}", def.id, def.label);
    }
}

fn detect_once() -> Option<wgopt_core::game_detect::GameCandidate> {
    let mut cpu = wgopt_core::game_detect::CpuSampler::new();
    let mut gpu = wgopt_core::gpu::GpuSampler::new();

    // GPU 计数器需要两次读数，先建立基线再采样
    let _ = cpu.sample();
    if let Some(g) = gpu.as_mut() {
        let _ = g.collect();
    }
    std::thread::sleep(std::time::Duration::from_millis(1200));

    let cpu_usage = cpu.sample();
    let gpu_usage = match gpu.as_mut() {
        Some(g) => g.collect(),
        None => Default::default(),
    };
    wgopt_core::game_detect::detect_game(&cpu_usage, &gpu_usage)
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

    let mut snap = wgopt_core::Snapshot::default();
    println!("开始优化，共 {} 项", cfg.enabled_items.len());
    let results = wgopt_core::engine::run_items(&cfg.enabled_items, None, &mut snap);
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

fn build_menu_state(shared: &Shared) -> MenuState {
    let status = shared.status_snapshot();
    let defs = wgopt_core::item_definitions();
    let (auto, items) = match shared.config.lock() {
        Ok(cfg) => (
            cfg.auto_optimize,
            defs.iter().map(|d| cfg.is_item_enabled(&d.id)).collect(),
        ),
        Err(_) => (true, vec![true; defs.len()]),
    };

    MenuState {
        enabled: status.enabled,
        auto,
        autostart: config::is_autostart_enabled(),
        optimized: status.phase == Phase::Optimized,
        status_line: status.status_line(),
        items,
    }
}

fn handle_choice(shared: &Shared, choice: MenuChoice) {
    match choice {
        MenuChoice::Command(TrayCommand::ToggleEnabled) => toggle_enabled(shared),
        MenuChoice::Command(TrayCommand::ToggleAuto) => toggle_auto(shared),
        MenuChoice::Command(TrayCommand::Start) => {
            shared.post_command(Command::Start);
        }
        MenuChoice::Command(TrayCommand::Stop) => {
            shared.post_command(Command::Stop);
        }
        MenuChoice::Command(TrayCommand::ToggleAutostart) => toggle_autostart(shared),
        MenuChoice::Command(TrayCommand::OpenConfig) => open_config_dir(),
        MenuChoice::Command(TrayCommand::Quit) => {
            shared.post_command(Command::Shutdown);
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

    // 关闭总开关时立刻还原已生效的改动，避免留下副作用
    if !now && shared.phase() == Phase::Optimized {
        shared.post_command(Command::Stop);
    }
}

fn toggle_auto(shared: &Shared) {
    let mut now = false;
    app::persist(shared, |cfg| {
        cfg.auto_optimize = !cfg.auto_optimize;
        now = cfg.auto_optimize;
    });
    logging::info(&format!("自动优化切换为 {}", now));
    shared.notify(
        "WGOpt",
        if now {
            "已开启：检测到游戏后自动优化"
        } else {
            "已关闭自动优化：仅提示"
        },
        NotifyKind::Info,
    );
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
