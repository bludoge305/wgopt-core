//! 手动验证：「游戏运行时自动优化」开关与后台检测联动。
//!
//! 检查三件事：
//! 1. 启动时该开关已关闭 → 后台一次检测都不做（日志无「检测到游戏」）
//! 2. 运行中关闭 → 立刻停用（日志出现「后台游戏检测已停用」）
//! 3. 重新开启 → 恢复检测并重建采样基线（日志出现「已启用（采样基线已重建）」）
//!
//! 运行：
//! ```text
//! cargo run --release --example detection_gate
//! ```
//!
//! 全程只改内存里的配置，不写 `%APPDATA%\WGOpt\config.json`，
//! 也不执行任何优化，跑完不影响真实环境。

use std::time::Duration;

use wgopt_core::app::{self, Command, Phase, Shared};
use wgopt_core::config::Config;
use wgopt_core::{logging, winapi};

/// 等够两个轮询周期，让守护线程有机会跑（或有机会「不跑」）。
const WAIT: Duration = Duration::from_millis(7000);

fn main() {
    let dir = std::env::temp_dir().join("wgopt-detection-gate");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    logging::init(&dir);
    let log_path = dir.join("wgopt.log");

    println!("=== 后台检测联动验证 ===");
    println!("日志：{}", log_path.display());

    // ---- 场景 1：启动时开关已关 ----
    let mut cfg = Config::default();
    cfg.enabled = true;
    cfg.auto_optimize = false;
    cfg.enabled_items = vec!["dns".to_string()];
    let shared = Shared::new(cfg);

    let worker = shared.clone();
    std::thread::spawn(move || app::run_guardian(worker));

    std::thread::sleep(WAIT);

    let log = read(&log_path);
    let started_off = log.contains("自动优化已关闭：本次启动不做游戏检测");
    let detected = log.contains("检测到游戏");
    let status = shared.status_snapshot();

    println!("\n[1] 启动时开关已关（等待 {} ms）", WAIT.as_millis());
    println!("    日志「本次启动不做游戏检测」: {started_off}");
    println!("    日志出现「检测到游戏」    : {detected}（应为 false）");
    println!("    状态：{}", status.status_line());
    println!("    游戏：{:?}（应为 None）", status.game.as_ref().map(|g| &g.name));

    // ---- 场景 2：运行中关闭 ----
    set_auto(&shared, true);
    shared.post_command(Command::ConfigChanged);
    std::thread::sleep(WAIT);
    let log = read(&log_path);
    let resumed = log.contains("后台游戏检测已启用（采样基线已重建）");

    println!("\n[2] 运行中重新开启");
    println!("    日志「已启用（采样基线已重建）」: {resumed}（应为 true）");

    set_auto(&shared, false);
    shared.post_command(Command::ConfigChanged);
    std::thread::sleep(Duration::from_millis(1500));
    let log = read(&log_path);
    let stopped = log.contains("自动优化已关闭：后台游戏检测已停用");

    println!("\n[3] 运行中再次关闭");
    println!("    日志「后台游戏检测已停用」      : {stopped}（应为 true）");

    let phase = shared.phase();
    println!("\n[收尾] 状态：{}，自动优化：{}",
        phase.label(),
        shared.config.lock().unwrap().auto_optimize);
    let _ = winapi::current_pid();

    // 关闭检测时若上一轮优化是自动触发的，应已就地还原回 Idle
    let ok = started_off && !detected && resumed && stopped && phase == Phase::Idle;
    println!("\n[结果] {}", if ok { "✓ 通过" } else { "✗ 失败" });

    let _ = std::fs::remove_dir_all(&dir);
    std::process::exit(if ok { 0 } else { 1 });
}

/// 只改内存里的配置，不落盘。
fn set_auto(shared: &Shared, on: bool) {
    if let Ok(mut cfg) = shared.config.lock() {
        cfg.auto_optimize = on;
    }
}

fn read(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}
