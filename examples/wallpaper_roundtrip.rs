//! 手动验证：Wallpaper Engine 的关闭与还原。
//!
//! 走的是托盘菜单用的同一对函数（`app::optimize_now` / `app::restore_now`），
//! 所以在同一个进程里能完整验证「快照记录 → 还原重启」这条链路。
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

/// 用 tasklist 查 Wallpaper Engine 是否在跑。
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

    // 只启用 wallpaper，避免其他优化项干扰判断
    let mut cfg = Config::default();
    cfg.enabled = true;
    cfg.auto_optimize = false;
    cfg.enabled_items = vec!["wallpaper".to_string()];
    cfg.notify_on_start = false;
    cfg.notify_on_stop = false;

    let shared = Shared::new(cfg);

    println!("\n[1] 执行优化（调用托盘「立即开始优化」同一函数）");
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

    println!("\n[2] 执行还原（调用托盘「停止优化并还原」同一函数）");
    let messages = app::restore_now(&shared, "验证脚本触发还原");
    for m in &messages {
        println!("    {m}");
    }

    // 给进程一点启动时间
    let mut running = None;
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(700));
        if let Some(line) = we_running() {
            running = Some(line);
            break;
        }
    }

    println!();
    match running {
        Some(line) => println!("[结果] ✓ WE 已重新启动: {line}"),
        None => {
            println!("[结果] ✗ WE 未能重新启动");
            std::process::exit(1);
        }
    }
}
