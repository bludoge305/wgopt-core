//! 验证还原逻辑的边界情况：
//! 1. 用户已经手动把 Wallpaper Engine 开回来了 —— 不应该再启动第二份
//! 2. 快照里的路径已失效（文件被删/移动）—— 不应该 panic，要给出可读错误
//! 3. 目标进程启动后立即退出 —— 不能报"已重新启动"
//! 4. 优化时 WE 原本未运行 —— 快照为空，还原不启动任何东西
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
    let mut failed = false;

    // ---------- 边界 1：用户已手动重开 ----------
    println!("[边界 1] 目标进程已在运行时，不应重复启动");

    let shared = Shared::new(Config {
        enabled_items: vec!["wallpaper".to_string()],
        notify_on_start: false,
        notify_on_stop: false,
        ..Config::default()
    });

    let before = we_pids();
    println!("    当前 WE 进程: {before:?}");
    if before.is_empty() {
        println!("    ! WE 未运行，本用例跳过");
    } else {
        // 构造一个指向当前正在运行的 WE 的快照，模拟"优化后用户又手动打开了"
        {
            let mut snap = shared.snapshot.lock().unwrap();
            snap.killed_programs.push(
                "D:\\Program Files\\Steam\\steamapps\\common\\wallpaper_engine\\wallpaper64.exe"
                    .to_string(),
            );
        }
        for m in app::restore_now(&shared, "边界1") {
            println!("    {m}");
        }
        std::thread::sleep(Duration::from_secs(2));
        let after = we_pids();
        let duplicated = after.len() > before.len();
        println!(
            "    {} 进程数 {} -> {}",
            if duplicated { "✗ 重复启动" } else { "✓ 未重复启动" },
            before.len(),
            after.len()
        );
        failed |= duplicated;
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
    println!("    {} 返回了可读的失败说明", if ok { "✓" } else { "✗" });
    failed |= !ok;

    // ---------- 边界 3：启动后立即退出 ----------
    println!("\n[边界 3] 目标程序启动后立即退出，不能报成功");

    // 复制一个「跑完就退出」的系统程序，并改成独一无二的名字：
    // 既保证名字不在当前进程列表里（不会走"已在运行"的短路），
    // 又能真的被 CreateProcess 启动后立刻结束。
    let unique = std::env::temp_dir().join(format!("wgopt_edge_exit_{}.exe", std::process::id()));
    let prepared = std::fs::copy(r"C:\Windows\System32\hostname.exe", &unique).is_ok();
    if !prepared {
        println!("    ! 无法准备临时程序，本用例跳过");
    } else {
        let snap = Snapshot {
            killed_programs: vec![unique.to_string_lossy().to_string()],
            ..Default::default()
        };
        let msgs = engine::restore(&snap);
        for m in &msgs {
            println!("    {m}");
        }
        let honest = msgs.iter().any(|m| m.contains("失败"));
        println!("    {} 没有把立即退出当成成功", if honest { "✓" } else { "✗" });
        failed |= !honest;
        let _ = std::fs::remove_file(&unique);
    }

    // ---------- 边界 4：原本未运行 ----------
    println!("\n[边界 4] 优化时 WE 原本未运行（空快照），还原不应启动任何东西");
    let before = we_pids().len();
    let msgs = engine::restore(&Snapshot::default());
    std::thread::sleep(Duration::from_secs(1));
    let after = we_pids().len();
    let quiet = msgs.is_empty() && after == before;
    println!(
        "    返回消息数 = {}，WE 进程数 {} -> {} {}",
        msgs.len(),
        before,
        after,
        if quiet { "✓" } else { "✗" }
    );
    failed |= !quiet;

    println!(
        "\n[汇总] {}",
        if failed { "✗ 有边界用例未通过" } else { "✓ 全部边界用例通过" }
    );
    if failed {
        std::process::exit(1);
    }
}
