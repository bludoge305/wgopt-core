//! 后台常驻守护：命令处理 + 3 秒游戏检测 + 自动优化/恢复。
//!
//! 原版这些逻辑在前端 JS 里（`startGamePolling` / `pollGame` /
//! `startRun` / `stopRun`）。没有界面后整体搬到 Rust 侧，输出改为
//! 更新共享状态，由托盘线程定时读取。
//!
//! 状态机：
//! ```text
//! Idle ──检测到游戏且自动优化开──▶ Optimizing ──完成──▶ Optimized
//!  ▲                                                    │
//!  └────────────── 游戏退出满 6 秒 ────────────────────┘
//! ```
//!
//! 「游戏运行时自动优化」是后台检测的开关，不只是"检测到要不要执行"：
//! 关闭后整个轮询停用——不采样 CPU/GPU、不识别、不提示、不随程序启动；
//! 重新开启时重建采样基线后恢复，与程序刚启动时一致。

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::config::{self, Config};
use crate::engine;
use crate::game_detect::{self, CpuSampler, GameCandidate};
use crate::gpu::GpuSampler;
use crate::item::{self, ItemResult};
use crate::logging;
use crate::snapshot::Snapshot;
use crate::tray::NotifyKind;

/// 检测轮询间隔，与原版一致。
const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// 游戏退出后的宽限时间，避免切窗口导致反复启停。
const EXIT_GRACE: Duration = Duration::from_secs(6);
/// 命令队列的等待粒度。
const TICK: Duration = Duration::from_millis(200);

/// 整体运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Optimizing,
    Optimized,
}

impl Phase {
    pub fn label(&self) -> &'static str {
        match self {
            Phase::Idle => "空闲",
            Phase::Optimizing => "优化中",
            Phase::Optimized => "已优化",
        }
    }
}

/// 托盘展示的运行状态。
#[derive(Debug, Clone)]
pub struct Status {
    pub phase: Phase,
    /// 总开关
    pub enabled: bool,
    /// 最近一次识别到的游戏
    pub game: Option<GameCandidate>,
    /// 最近一轮各项结果
    pub results: Vec<ItemResult>,
    /// 最近一次状态变化说明
    pub message: String,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            enabled: true,
            game: None,
            results: Vec::new(),
            message: String::new(),
        }
    }
}

impl Status {
    /// 托盘悬浮提示。
    pub fn tooltip(&self) -> String {
        if !self.enabled {
            return "WGOpt · 已停用".to_string();
        }
        let mut s = format!("WGOpt · {}", self.phase.label());
        if let Some(g) = &self.game {
            s.push_str(" · ");
            s.push_str(&g.label());
        }
        s
    }

    /// 菜单顶部状态行，含已完成项计数。
    pub fn status_line(&self) -> String {
        if !self.enabled {
            return "状态：已停用（总开关关闭）".into();
        }
        let done = self.results.iter().filter(|r| r.is_completed()).count();
        let total = self.results.len();
        let game = self
            .game
            .as_ref()
            .map(|g| format!(" · {}", g.label()))
            .unwrap_or_default();

        match self.phase {
            Phase::Idle => format!("状态：待机{game}"),
            Phase::Optimizing => format!("状态：优化中 {done}/{total}{game}"),
            Phase::Optimized => format!("状态：已优化 {done}/{total}{game}"),
        }
    }
}

/// 一条待弹出的系统通知。
#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub kind: NotifyKind,
}

/// 后台线程要执行的命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// 立即优化
    Start,
    /// 停止优化并还原
    Stop,
    /// 配置已改：立刻重新判定检测开关，不等下一个轮询周期
    ConfigChanged,
    /// 退出：还原后结束进程
    Shutdown,
}

/// 运行期共享状态。
pub struct Shared {
    pub config: Mutex<Config>,
    pub snapshot: Mutex<Snapshot>,
    pub status: Mutex<Status>,
    /// 待弹出的通知队列，托盘线程消费
    notifications: Mutex<VecDeque<Notification>>,
    /// 命令队列与唤醒信号
    commands: Mutex<VecDeque<Command>>,
    wake: Condvar,
    /// 退出标志
    quit: std::sync::atomic::AtomicBool,
}

impl Shared {
    pub fn new(config: Config) -> Arc<Self> {
        let status = Status {
            enabled: config.enabled,
            ..Default::default()
        };
        Arc::new(Self {
            config: Mutex::new(config),
            snapshot: Mutex::new(Snapshot::default()),
            status: Mutex::new(status),
            notifications: Mutex::new(VecDeque::new()),
            commands: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            quit: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// 把配置里的总开关同步到状态。
    pub fn sync_enabled(&self) {
        let enabled = self.config.lock().map(|c| c.enabled).unwrap_or(true);
        if let Ok(mut st) = self.status.lock() {
            st.enabled = enabled;
        }
    }

    pub fn phase(&self) -> Phase {
        self.status.lock().map(|s| s.phase).unwrap_or(Phase::Idle)
    }

    pub fn status_snapshot(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set_phase(&self, phase: Phase, message: impl Into<String>) {
        if let Ok(mut st) = self.status.lock() {
            st.phase = phase;
            st.message = message.into();
        }
    }

    /// 入队一条通知。
    pub fn notify(&self, title: impl Into<String>, body: impl Into<String>, kind: NotifyKind) {
        if let Ok(mut q) = self.notifications.lock() {
            // 队列过长说明托盘线程卡住或刷屏，丢弃最旧的
            if q.len() >= 16 {
                q.pop_front();
            }
            q.push_back(Notification {
                title: title.into(),
                body: body.into(),
                kind,
            });
        }
    }

    /// 取走所有待弹通知。
    pub fn drain_notifications(&self) -> Vec<Notification> {
        self.notifications
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }

    /// 投递命令并唤醒后台线程。
    pub fn post_command(&self, cmd: Command) {
        if let Ok(mut q) = self.commands.lock() {
            q.push_back(cmd);
        }
        self.wake.notify_one();
    }

    fn take_command(&self) -> Option<Command> {
        self.commands.lock().ok().and_then(|mut q| q.pop_front())
    }

    /// 等待一段时间，期间有命令到达会提前唤醒。
    fn wait(&self, timeout: Duration) {
        if let Ok(guard) = self.commands.lock() {
            let _ = self
                .wake
                .wait_timeout_while(guard, timeout, |q| q.is_empty());
        }
    }

    pub fn quit_requested(&self) -> bool {
        self.quit.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn request_quit(&self) {
        self.quit.store(true, std::sync::atomic::Ordering::SeqCst);
        self.wake.notify_all();
    }
}

/// 后台检测闸门。
///
/// 「游戏运行时自动优化」同时就是后台检测的开关：关掉它以后轮询线程
/// 不采样 CPU/GPU、不识别游戏、也不提示。总开关关闭时同理。
fn detection_enabled(enabled: bool, auto: bool) -> bool {
    enabled && auto
}

/// 建立采样基线：GPU 计数器与 CPU 差值都要两次读数才有意义。
/// 检测从停用切回启用时必须重来一遍，否则第一次读数是跨越整段停用
/// 时间的累计值，游戏会被一次性算成满占用。
fn prime_detection(cpu: &mut CpuSampler, gpu: &mut Option<GpuSampler>) {
    let _ = cpu.sample();
    if gpu.is_none() {
        *gpu = GpuSampler::new();
    }
    if let Some(g) = gpu.as_mut() {
        let _ = g.collect();
    }
}

/// 后台守护循环（阻塞调用线程）。
pub fn run_guardian(shared: Arc<Shared>) -> ! {
    let mut cpu = CpuSampler::new();
    // GPU 采集器按需创建：检测停用时连 PDH 查询都不必打开
    let mut gpu: Option<GpuSampler> = None;
    let mut exit_at: Option<Instant> = None;
    let mut last_game_pid: Option<u32> = None;
    let mut next_poll = Instant::now();
    // 采样基线是否已经建立。检测停用期间保持 false，恢复时重新预热。
    let mut detecting = false;
    // 当前生效的优化是不是「检测到游戏」自动触发的。关闭自动优化时只还原
    // 这一种；手点「立即开始优化」的结果不归这个开关管。
    let mut auto_applied = false;

    logging::info("后台守护已启动（轮询间隔 3 秒）");

    // 自动优化关闭时一次采样都不做——进程启动后不该有任何检测动作
    let initial = match shared.config.lock() {
        Ok(cfg) => (cfg.enabled, cfg.auto_optimize),
        Err(_) => (true, true),
    };
    if detection_enabled(initial.0, initial.1) {
        prime_detection(&mut cpu, &mut gpu);
        detecting = true;
    } else {
        logging::info("自动优化已关闭：本次启动不做游戏检测");
    }

    loop {
        // ---- 1. 先处理命令，让用户操作即时生效 ----
        // 配置变更后不等下一个轮询周期，立刻重跑一轮判定。
        let mut recheck = false;
        while let Some(cmd) = shared.take_command() {
            match cmd {
                Command::Start => {
                    let game = shared.status_snapshot().game.map(|g| g.name);
                    auto_applied = false;
                    optimize_now(&shared, game);
                }
                Command::Stop => {
                    auto_applied = false;
                    restore_now(&shared, "已手动停止优化，可还原项已恢复");
                }
                Command::ConfigChanged => recheck = true,
                Command::Shutdown => {
                    if shared.phase() == Phase::Optimized {
                        restore_now(&shared, "程序退出，已自动恢复");
                    }
                    logging::info("收到退出命令，进程结束");
                    shared.request_quit();
                    exit_process();
                }
            }
        }

        if shared.quit_requested() {
            exit_process();
        }

        // 配置刚变过：把下一次轮询提前到现在，让开关立即生效
        if recheck {
            next_poll = Instant::now();
        }

        // ---- 2. 到点才做游戏检测 ----
        let now = Instant::now();
        if now < next_poll {
            shared.wait(TICK.min(next_poll - now));
            continue;
        }
        next_poll = now + POLL_INTERVAL;

        let (enabled, auto) = match shared.config.lock() {
            Ok(cfg) => (cfg.enabled, cfg.auto_optimize),
            Err(_) => (true, true),
        };
        let phase = shared.phase();

        // 总开关关闭：完全不检测，图标保持"已停用"
        if !enabled {
            if phase != Phase::Idle {
                shared.set_phase(Phase::Idle, "总开关已关闭");
            }
            last_game_pid = None;
            exit_at = None;
            detecting = false;
            auto_applied = false;
            continue;
        }

        // 自动优化关闭：后台检测整体停用。
        // 不采样 CPU/GPU、不识别、不提示；清掉上一轮的识别结果，让托盘
        // 不再显示游戏名。
        if !detection_enabled(enabled, auto) {
            if detecting {
                logging::info("自动优化已关闭：后台游戏检测已停用");
                detecting = false;
                // PDH 查询句柄一并释放，停用期间不占采集资源
                gpu = None;
            }
            last_game_pid = None;
            exit_at = None;
            // 自动触发的那轮优化要跟着还原：检测一停，就再没人观察游戏何时
            // 退出，快照里的优先级、服务、Wallpaper Engine 会一直留着。
            // 手点「立即开始优化」的结果不归这个开关管，保持不动。
            if auto_applied && shared.phase() == Phase::Optimized {
                restore_now(&shared, "自动优化已关闭，已恢复");
                auto_applied = false;
            }
            let mut cleared = false;
            if let Ok(mut st) = shared.status.lock() {
                cleared = st.game.take().is_some();
                if cleared {
                    st.message = "自动优化已关闭，后台检测已停用".into();
                }
            }
            if cleared {
                logging::info("自动优化已关闭：已清除上一轮的游戏识别结果");
            }
            continue;
        }

        // 从"检测停用"切回"检测启用"：先重建采样基线。
        // CPU 采样差值与 GPU 计数器都依赖两次读数，停用期间基线是旧的，
        // 直接拿去算会得到跨了整段停用时间的假占用（游戏可能一次性满分）。
        if !detecting {
            prime_detection(&mut cpu, &mut gpu);
            detecting = true;
            logging::info("后台游戏检测已启用（采样基线已重建）");
            next_poll = Instant::now() + POLL_INTERVAL;
            continue;
        }

        // 正在优化时跳过本轮
        if phase == Phase::Optimizing {
            continue;
        }

        let cpu_usage = cpu.sample();
        let gpu_usage = match gpu.as_mut() {
            Some(g) => g.collect(),
            None => Default::default(),
        };

        match game_detect::detect_game(&cpu_usage, &gpu_usage) {
            Some(game) => {
                let changed = last_game_pid != Some(game.pid);
                exit_at = None;
                last_game_pid = Some(game.pid);
                if let Ok(mut st) = shared.status.lock() {
                    st.game = Some(game.clone());
                }

                if !changed {
                    continue;
                }

                let label = game.label();
                logging::info(&format!(
                    "检测到游戏：{label}（{} · PID {}，判定得分 {}）",
                    game.name, game.pid, game.score
                ));

                shared.notify(
                    "检测到游戏",
                    format!("{label}，正在自动优化"),
                    NotifyKind::Info,
                );
                auto_applied = true;
                optimize_now(&shared, Some(game.name.clone()));
            }
            None => {
                if last_game_pid.is_none() {
                    continue;
                }

                // 游戏消失：先等宽限期，避免切窗口导致反复启停
                let start = *exit_at.get_or_insert_with(Instant::now);
                if start.elapsed() < EXIT_GRACE {
                    continue;
                }

                let had_game = shared
                    .status_snapshot()
                    .game
                    .map(|g| g.label())
                    .unwrap_or_default();
                last_game_pid = None;
                exit_at = None;
                if let Ok(mut st) = shared.status.lock() {
                    st.game = None;
                }
                logging::info(&format!("游戏已退出：{had_game}"));

                if shared.phase() == Phase::Optimized {
                    restore_now(&shared, "游戏已退出，已自动恢复");
                }
                auto_applied = false;
            }
        }
    }
}

/// 立即执行一轮优化（当前启用的所有项）。
pub fn optimize_now(shared: &Shared, game: Option<String>) -> Vec<ItemResult> {
    if !shared.config.lock().map(|c| c.enabled).unwrap_or(true) {
        logging::info("总开关已关闭，忽略优化请求");
        return Vec::new();
    }
    {
        // 抢占状态，拦住重复进入
        let mut st = match shared.status.lock() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        if st.phase == Phase::Optimizing {
            return Vec::new();
        }
        st.phase = Phase::Optimizing;
        st.message = "正在优化".into();
    }

    let (ids, notify) = match shared.config.lock() {
        Ok(cfg) => (cfg.enabled_items.clone(), cfg.notify_on_start),
        Err(_) => return Vec::new(),
    };

    if ids.is_empty() {
        shared.set_phase(Phase::Idle, "没有启用任何优化项");
        return Vec::new();
    }

    logging::info(&format!("开始优化，共 {} 项", ids.len()));
    let results = {
        let mut snap = match shared.snapshot.lock() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        // 每次全量优化前重置快照，避免同一 pid 重复累积。但「本程序关闭、
        // 尚未还原」的程序必须带过来：连续两轮优化之间没有经过 restore，
        // 清掉它等于把上一轮关掉的 Wallpaper Engine 永久留在关闭状态。
        snap.reset_keeping_pending_programs();
        engine::run_items(&ids, game.as_deref(), &mut snap)
    };

    for r in &results {
        let level = match r.status {
            item::Status::Done => "完成",
            item::Status::Skip => "跳过",
            item::Status::Fail => "失败",
        };
        logging::info(&format!("{level}：{}（{}ms）· {}", r.id, r.elapsed_ms, r.detail));
    }

    let total = results.len();
    let done = results.iter().filter(|r| r.is_completed()).count();
    let failed = results.iter().filter(|r| r.status == item::Status::Fail).count();

    if let Ok(mut st) = shared.status.lock() {
        st.phase = Phase::Optimized;
        st.results = results.clone();
        st.message = format!("已完成 {done}/{total}");
    }

    if notify {
        let body = if failed > 0 {
            format!("已完成 {done}/{total}，其中 {failed} 项失败")
        } else {
            format!("已完成 {done}/{total} 项优化")
        };
        shared.notify(
            "优化完成",
            body,
            if failed > 0 { NotifyKind::Warn } else { NotifyKind::Info },
        );
    }
    results
}

/// 停止优化并还原快照。
pub fn restore_now(shared: &Shared, reason: &str) -> Vec<String> {
    let messages = {
        let mut snap = match shared.snapshot.lock() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let messages = engine::restore(&snap);
        *snap = Snapshot::default();
        messages
    };

    for m in &messages {
        logging::info(m);
    }

    let notify = shared.config.lock().map(|c| c.notify_on_stop).unwrap_or(true);

    if let Ok(mut st) = shared.status.lock() {
        st.phase = Phase::Idle;
        st.results.clear();
        st.message = reason.to_string();
    }

    if notify {
        shared.notify("已停止优化", reason, NotifyKind::Info);
    }
    messages
}

/// 修改并保存配置。
pub fn persist(shared: &Shared, mutate: impl FnOnce(&mut Config)) {
    let updated = {
        let mut cfg = match shared.config.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        mutate(&mut cfg);
        cfg.normalize();
        cfg.clone()
    };
    if let Err(e) = config::save(&updated) {
        logging::error(&format!("保存配置失败：{e}"));
    }
    shared.sync_enabled();
}

fn exit_process() -> ! {
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 「游戏运行时自动优化」同时是后台检测的开关：
    /// 关闭它以后不得再采样、识别或提示。
    #[test]
    fn auto_off_disables_detection() {
        assert!(!detection_enabled(true, false));
    }

    /// 总开关关闭时自动优化是否开启都不检测。
    #[test]
    fn master_off_disables_detection() {
        assert!(!detection_enabled(false, true));
        assert!(!detection_enabled(false, false));
    }

    /// 两个开关都开时保持原有行为。
    #[test]
    fn both_on_keeps_detection() {
        assert!(detection_enabled(true, true));
    }
}
