//! 手动验证：Wallpaper Engine 的关闭与还原。
//!
//! 走的是托盘菜单用的同一对函数（`app::optimize_now` / `app::restore_now`），
//! 所以在同一个进程里能完整验证「快照记录 → 还原重启」这条链路。
//!
//! 场景 A 复现线上故障：默认配置里 `background` 排在 `wallpaper` 之前，
//! 而后台白名单也包含 Wallpaper Engine，于是实际是 `background` 先结束它，
//! `wallpaper` 随后只看得到"未检测到"。这条路径必须同样记录路径并在还原
//! 时重新拉起。
//!
//! 运行：
//! ```text
//! cargo run --release --example wallpaper_roundtrip
//! ```
//!
//! 需要 Wallpaper Engine 正在运行才有意义（否则 wallpaper 项会被跳过）。

use std::process::Command;
use std::time::Duration;

use wgopt_core::app::{self, Shared};
use wgopt_core::config::Config;

/// 用 tasklist 查 Wallpaper Engine 的主进程。
fn we_running() -> Option<String> {
    let out = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq wallpaper64.exe", "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    if text.contains("wallpaper64.exe") {
        Some(text.lines().next().unwrap_or("").trim().to_string())
    } else {
        None
    }
}

fn main() {
    println!("=== Wallpaper Engine 关闭/还原验证 ===\n");

    match we_running() {
        Some(line) => println!("[基线] WE 正在运行: {line}"),
        None => {
            println!("[基线] WE 未运行，跳过验证（优化项会直接 skip）");
            return;
        }
    }

    // 只启用 wallpaper，验证该项自身的记录与还原
    let shared = Shared::new(config_with(&["wallpaper"]));

    println!("\n[1] 执行优化（仅 wallpaper 项）");
    let results = app::optimize_now(&shared, None);
    for r in &results {
        println!("    {:<10} {:?}  {}", r.id, r.status, r.detail);
    }

    std::thread::sleep(Duration::from_secs(2));
    match we_running() {
        Some(_) => println!("    ✗ WE 仍在运行（应当已被关闭）"),
        None => println!("    ✓ WE 已被关闭"),
    }

    {
        let snap = shared.snapshot.lock().unwrap();
        println!("    快照记录 killed_programs = {:?}", snap.killed_programs);
    }

    println!("\n[2] 执行还原");
    for m in app::restore_now(&shared, "验证脚本触发还原") {
        println!("    {m}");
    }

    match wait_running() {
        Some(line) => println!("\n[结果] ✓ WE 已重新启动: {line}"),
        None => {
            println!("\n[结果] ✗ WE 未能重新启动");
            std::process::exit(1);
        }
    }

    // ---------- 场景 A：background 先结束（线上实际走的路径）----------
    println!("\n[3] 场景 A：background + wallpaper 全量优化（后台项先命中）");
    // 保持默认顺序：background 在 wallpaper 之前
    let shared = Shared::new(config_with(&["background", "wallpaper"]));

    for r in app::optimize_now(&shared, None) {
        println!("    {:<10} {:?}  {}", r.id, r.status, r.detail);
    }
    std::thread::sleep(Duration::from_secs(2));

    let recorded = {
        let snap = shared.snapshot.lock().unwrap();
        snap.killed_programs.clone()
    };
    println!("    快照记录 killed_programs = {recorded:?}");
    let recorded_ok = !recorded.is_empty();
    println!(
        "    {} 后台项结束时也记下了路径",
        if recorded_ok { "✓" } else { "✗" }
    );

    println!("\n[4] 还原");
    for m in app::restore_now(&shared, "场景 A 还原") {
        println!("    {m}");
    }
    let restored = wait_running();
    match &restored {
        Some(line) => println!("[结果] ✓ WE 已重新启动: {line}"),
        None => println!("[结果] ✗ WE 未能重新启动"),
    }

    // ---------- 场景 B：连续两轮优化，中间不还原 ----------
    // 第二轮开始时 WE 已被第一轮关掉，进程查不到了。若快照被整份重置，
    // 路径就永久丢失，还原将无事可做。
    println!("\n[5] 场景 B：连续两轮优化不还原（快照不得丢失待还原记录）");
    let shared = Shared::new(config_with(&["wallpaper"]));

    app::optimize_now(&shared, None);
    std::thread::sleep(Duration::from_secs(1));
    app::optimize_now(&shared, None);
    std::thread::sleep(Duration::from_secs(1));

    let recorded = {
        let snap = shared.snapshot.lock().unwrap();
        snap.killed_programs.clone()
    };
    println!("    两轮之后 killed_programs = {recorded:?}");
    let kept = !recorded.is_empty();
    println!("    {} 第二轮没有丢掉关闭记录", if kept { "✓" } else { "✗" });

    println!("\n[6] 还原");
    for m in app::restore_now(&shared, "场景 B 还原") {
        println!("    {m}");
    }
    let restored_b = wait_running();
    match &restored_b {
        Some(line) => println!("[结果] ✓ WE 已重新启动: {line}"),
        None => println!("[结果] ✗ WE 未能重新启动"),
    }

    if !(recorded_ok && restored.is_some() && kept && restored_b.is_some()) {
        std::process::exit(1);
    }
}

/// 验证用的配置：只启用指定项，关掉自动优化与通知，避免干扰判断。
fn config_with(items: &[&str]) -> Config {
    Config {
        enabled: true,
        auto_optimize: false,
        enabled_items: items.iter().map(|s| s.to_string()).collect(),
        notify_on_start: false,
        notify_on_stop: false,
        ..Config::default()
    }
}

/// 最多等 7 秒，看 WE 主进程是否出现。
fn wait_running() -> Option<String> {    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(700));
        if let Some(line) = we_running() {
            return Some(line);
        }
    }
    None
}
