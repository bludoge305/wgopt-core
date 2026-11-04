//! 验证还原逻辑的两个边界情况：
//! 1. 用户已经手动把 Wallpaper Engine 开回来了 —— 不应该再启动第二份
//! 2. 快照里的路径已失效（文件被删/移动）—— 不应该 panic，要给出可读错误
//!
//! 运行：cargo run --release --example wallpaper_restore_edges

use std::process::Command;
use std::time::Duration;

use wgopt_core::app::{self, Shared};
use wgopt_core::config::Config;
use wgopt_core::engine;
use wgopt_core::snapshot::Snapshot;

fn we_pids() -> Vec<String> {
    let Ok(out) = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq wallpaper64.exe", "/FO", "CSV", "/NH"])
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter(|l| l.contains("wallpaper64.exe"))
        .filter_map(|l| l.split("\",\"").nth(1).map(|s| s.to_string()))
        .collect()
}

fn main() {
    println!("=== 还原逻辑边界验证 ===\n");

    // ---------- 边界 1：用户已手动重开 ----------
    println!("[边界 1] 目标进程已在运行时，不应重复启动");

    let mut cfg = Config::default();
    cfg.enabled_items = vec!["wallpaper".to_string()];
    cfg.notify_on_start = false;
    cfg.notify_on_stop = false;
    let shared = Shared::new(cfg);

    let before = we_pids();
    println!("    当前 WE 进程: {before:?}");
    if before.is_empty() {
        println!("    ! WE 未运行，本用例跳过");
    } else {
        // 构造一个指向当前正在运行的 WE 的快照，模拟"优化后用户又手动打开了"
        let path = "D:\\Program Files\\Steam\\steamapps\\common\\wallpaper_engine\\wallpaper64.exe";
        {
            let mut snap = shared.snapshot.lock().unwrap();
            snap.killed_programs.push(path.to_string());
        }
        let msgs = app::restore_now(&shared, "边界1");
        for m in &msgs {
            println!("    {m}");
        }
        std::thread::sleep(Duration::from_secs(2));
        let after = we_pids();
        println!("    之后 WE 进程: {after:?}");
        let duplicated = after.len() > before.len();
        println!(
            "    {} 进程数 {} -> {}",
            if duplicated { "✗ 重复启动" } else { "✓ 未重复启动" },
            before.len(),
            after.len()
        );
    }

    // ---------- 边界 2：路径已失效 ----------
    println!("\n[边界 2] 快照里的路径不存在时，应给出可读错误而不是崩溃");

    let snap = Snapshot {
        // 用不存在的名字，避开"已在运行"的短路分支，真正走到启动失败
        killed_programs: vec![r"C:\NoSuchDir\no_such_wgopt_proc.exe".to_string()],
        ..Default::default()
    };
    let msgs = engine::restore(&snap);
    for m in &msgs {
        println!("    {m}");
    }
    let ok = msgs.iter().any(|m| m.contains("失败"));
    println!(
        "    {} 返回了可读的失败说明",
        if ok { "✓" } else { "✗" }
    );

    // ---------- 边界 3：空快照 ----------
    println!("\n[边界 3] 空快照不应产生任何消息");
    let msgs = engine::restore(&Snapshot::default());
    println!("    返回消息数 = {} {}", msgs.len(), if msgs.is_empty() { "✓" } else { "✗" });
}
