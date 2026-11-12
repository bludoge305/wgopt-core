//! 验证「用户已手动恢复 Wallpaper Engine」时不重复启动。
//!
//! 做法：让程序先关闭 WE 并记下快照，然后在还原之前手动把 WE 开回来，
//! 再走还原流程。还原应识别出它已在运行，不再启动第二份。
//!
//! 运行：cargo run --release --example wallpaper_manual_recover

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use wgopt_core::app::{self, Shared};
use wgopt_core::config::Config;

const WE_DIR: &str = r"D:\Program Files\Steam\steamapps\common\wallpaper_engine";

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

fn wait_we(timeout_secs: u64) -> Vec<String> {
    for _ in 0..(timeout_secs * 5) {
        let pids = we_pids();
        if !pids.is_empty() {
            return pids;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Vec::new()
}

fn main() {
    println!("=== 用户手动恢复时不重复启动 ===\n");

    let exe = PathBuf::from(WE_DIR).join("wallpaper64.exe");
    if !exe.exists() {
        println!(
            "! 找不到 {}\n  本机未装 Wallpaper Engine，跳过验证",
            exe.display()
        );
        std::process::exit(2);
    }

    // 基线：确保 WE 正在运行
    if wait_we(1).is_empty() {
        println!("[基线] WE 未运行，先启动");
        let _ = Command::new(&exe).current_dir(WE_DIR).spawn();
        if wait_we(15).is_empty() {
            println!("! WE 起不来，跳过验证");
            std::process::exit(2);
        }
    }
    let base = we_pids();
    println!("[基线] WE 正在运行: {base:?}");

    let shared = Shared::new(Config {
        enabled: true,
        enabled_items: vec!["wallpaper".to_string()],
        notify_on_start: false,
        notify_on_stop: false,
        ..Config::default()
    });

    println!("\n[1] 优化：关闭 WE 并记录路径");
    for r in app::optimize_now(&shared) {
        println!("    {:<10} {:?}  {}", r.id, r.status, r.detail);
    }
    std::thread::sleep(Duration::from_secs(2));
    if !we_pids().is_empty() {
        println!("    ✗ WE 未被关闭，用例无法继续");
        std::process::exit(1);
    }
    let recorded = {
        let snap = shared.snapshot.lock().unwrap();
        snap.killed_programs.clone()
    };
    println!("    ✓ WE 已关闭，快照 = {recorded:?}");

    println!("\n[2] 模拟用户手动把 WE 开回来");
    let _ = Command::new(&exe).current_dir(WE_DIR).spawn();
    let manual = wait_we(15);
    if manual.is_empty() {
        println!("    ✗ 手动启动失败，用例无法继续");
        std::process::exit(1);
    }
    println!("    ✓ 用户已手动恢复，进程: {manual:?}");

    println!("\n[3] 执行还原");
    let messages = app::restore_now(&shared, "用户手动恢复场景");
    for m in &messages {
        println!("    {m}");
    }

    std::thread::sleep(Duration::from_secs(3));
    let after = we_pids();
    println!("    还原后 WE 进程: {after:?}");

    let skipped = messages.iter().any(|m| m.contains("已在运行"));
    let no_duplicate = after.len() <= 1;
    println!(
        "    {} 识别出已在运行",
        if skipped { "✓" } else { "✗" }
    );
    println!(
        "    {} 未启动第二份（{} -> {}）",
        if no_duplicate { "✓" } else { "✗" },
        manual.len(),
        after.len()
    );

    if !(skipped && no_duplicate) {
        std::process::exit(1);
    }
    println!("\n[汇总] ✓ 手动恢复场景行为正确");
}
