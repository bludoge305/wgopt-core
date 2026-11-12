//! 验证「Wallpaper Engine 原本未运行」时的完整优化/还原流程：
//! 优化项应跳过、快照为空、还原不启动任何东西。
//!
//! 运行前请先退出 Wallpaper Engine。
//! 运行：cargo run --release --example wallpaper_not_running

use std::process::Command;
use std::time::Duration;

use wgopt_core::app::{self, Shared};
use wgopt_core::config::Config;

fn we_pids() -> Vec<String> {
    let Ok(out) = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq wallpaper64.exe", "/FO", "CSV", "/NH"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains("wallpaper64.exe"))
        .filter_map(|l| l.split("\",\"").nth(1).map(|s| s.to_string()))
        .collect()
}

fn main() {
    println!("=== 原本未运行时的优化/还原验证 ===\n");

    if !we_pids().is_empty() {
        println!("! Wallpaper Engine 正在运行，请先退出后重试（本用例验证的是未运行场景）");
        std::process::exit(2);
    }
    println!("[基线] WE 未运行 ✓");

    let shared = Shared::new(Config {
        enabled: true,
        // 两项都可能命中 WE，默认顺序下 background 在前
        enabled_items: vec!["background".to_string(), "wallpaper".to_string()],
        notify_on_start: false,
        notify_on_stop: false,
        ..Config::default()
    });

    println!("\n[1] 优化（两个可能命中 WE 的项）");
    for r in app::optimize_now(&shared) {
        println!("    {:<10} {:?}  {}", r.id, r.status, r.detail);
    }

    let snap_empty = {
        let snap = shared.snapshot.lock().unwrap();
        println!("    killed_programs = {:?}", snap.killed_programs);
        snap.killed_programs.is_empty() && snap.is_empty()
    };
    println!(
        "    {} 快照为空（没有可还原的关闭记录）",
        if snap_empty { "✓" } else { "✗" }
    );

    println!("\n[2] 还原：不应凭空启动 WE");
    let messages = app::restore_now(&shared, "未运行场景还原");
    for m in &messages {
        println!("    {m}");
    }
    std::thread::sleep(Duration::from_secs(3));
    let pids = we_pids();
    let quiet = messages.is_empty() && pids.is_empty();
    println!(
        "    返回消息数 = {}，WE 进程 = {pids:?} {}",
        messages.len(),
        if quiet { "✓" } else { "✗" }
    );

    if !(snap_empty && quiet) {
        std::process::exit(1);
    }
    println!("\n[汇总] ✓ 未运行场景行为正确");
}
