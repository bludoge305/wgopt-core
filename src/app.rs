//! 后台工作线程：命令处理、执行优化与还原。
//!
//! 程序没有游戏检测：优化只在用户明确要求时执行（托盘菜单或全局快捷键），
//! 由本模块的工作线程跑，避免阻塞消息循环（一轮优化里停服务、重启
//! Wallpaper Engine 可能耗到十几秒）。
//!
//! 状态机：
//! ```text
//! Idle ──Start 命令──▶ Optimizing ──完成──▶ Optimized
//!  ▲                                          │
//!  └────────────── Stop 命令 ────────────────┘
//! ```

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::config::{self, Config};
use crate::engine;
use crate::item::{self, ItemResult};
use crate::logging;
use crate::snapshot::Snapshot;
use crate::tray::NotifyKind;

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
    /// 最近一轮各项结果
    pub results: Vec<ItemResult>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            enabled: true,
            results: Vec::new(),
        }
    }
}

impl Status {
    /// 托盘悬浮提示。
    pub fn tooltip(&self) -> String {
        if !self.enabled {
            return "WGOpt · 已停用".to_string();
        }
        format!("WGOpt · {}", self.phase.label())
    }

    /// 菜单顶部状态行，含已完成项计数。
    pub fn status_line(&self) -> String {
        if !self.enabled {
            return "状态：已停用（总开关关闭）".into();
        }
        let done = self.results.iter().filter(|r| r.is_completed()).count();
        let total = self.results.len();

        match self.phase {
            Phase::Idle => "状态：待机".to_string(),
            Phase::Optimizing => format!("状态：优化中 {done}/{total}"),
            Phase::Optimized => format!("状态：已优化 {done}/{total}"),
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
}

/// 「优化状态切换」此刻能做什么。
///
/// 「立即开始优化」与「停止优化并还原」不是两个独立功能，而是同一个开关的
/// 两个方向。菜单文案、菜单可用性、快捷键行为全部由 [`Shared::toggle_action`]
/// 这一个判定产出，避免三处各写一套条件而互相漂移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleAction {
    /// 未优化：可执行「立即开始优化」
    Start,
    /// 已优化：可执行「停止优化并还原」
    Stop,
    /// 正在优化：两个方向都不可操作，等这一轮结束
    Busy,
    /// 总开关关闭：不优化也不还原
    Disabled,
}

impl ToggleAction {
    /// 菜单/按钮文案。
    pub fn label(self) -> &'static str {
        match self {
            ToggleAction::Start => "立即开始优化",
            ToggleAction::Stop => "停止优化并还原",
            ToggleAction::Busy => "优化进行中…",
            ToggleAction::Disabled => "立即开始优化",
        }
    }

    /// 是否可点击（可点击才允许快捷键触发）。
    pub fn is_available(self) -> bool {
        matches!(self, ToggleAction::Start | ToggleAction::Stop)
    }
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
    /// 退出标志：主线程结束时置位，让工作线程不再投递新动作
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

    /// 当前的优化状态切换动作。**菜单与快捷键都必须走这里**，不要另写判断。
    pub fn toggle_action(&self) -> ToggleAction {
        // 总开关关闭时既不做优化也不做还原，与工作线程的处理一致
        let enabled = self.config.lock().map(|c| c.enabled).unwrap_or(true);
        if !enabled {
            return ToggleAction::Disabled;
        }
        match self.phase() {
            Phase::Idle => ToggleAction::Start,
            Phase::Optimized => ToggleAction::Stop,
            Phase::Optimizing => ToggleAction::Busy,
        }
    }

    /// 判定并投递切换命令，返回判定结果与实际投递的命令。
    ///
    /// 判定与投递在同一个函数里，调用方拿不到"判断了但不投递"或
    /// "投递了但没判断"的中间态——菜单点击与快捷键触发都只调这一个。
    pub fn request_toggle(&self) -> (ToggleAction, Option<Command>) {
        let action = self.toggle_action();
        let command = match action {
            ToggleAction::Start => Some(Command::Start),
            ToggleAction::Stop => Some(Command::Stop),
            ToggleAction::Busy | ToggleAction::Disabled => None,
        };
        if let Some(cmd) = command {
            self.post_command(cmd);
        }
        (action, command)
    }

    pub fn status_snapshot(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// 快照里是否还有未还原的改动。
    ///
    /// 状态为「待机」不代表系统干净：优化跑到一半时关掉开关，phase 可能
    /// 还没到「已优化」，但进程优先级、服务、Wallpaper Engine 已经动过了。
    pub fn has_restorable_changes(&self) -> bool {
        self.snapshot.lock().map(|s| !s.is_empty()).unwrap_or(false)
    }

    /// 当前快照副本，供排障与测试观察「到底动了什么」。
    pub fn snapshot_copy(&self) -> Snapshot {
        self.snapshot.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set_phase(&self, phase: Phase) {
        if let Ok(mut st) = self.status.lock() {
            st.phase = phase;
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

    /// 通知工作线程停止工作。**不在这里还原**：还原统一由主线程在
    /// 消息循环结束后按 [`Shared::shutdown`] 做一次，避免两个线程各还原
    /// 一遍，把对方刚恢复的优先级再改回去。
    pub fn request_quit(&self) {
        self.quit.store(true, std::sync::atomic::Ordering::SeqCst);
        self.wake.notify_all();
    }

    /// 程序退出前的收尾：按配置还原可还原项。
    ///
    /// 还原与否看快照而不是 `phase`：优化跑到一半退出时 phase 还没到
    /// 「已优化」，但进程优先级、服务、Wallpaper Engine 已经动过了。
    pub fn shutdown(&self) {
        self.request_quit();
        let restore = self
            .config
            .lock()
            .map(|c| c.restore_on_exit)
            .unwrap_or(true);
        if !restore {
            logging::info("restore_on_exit 已关闭，退出时不还原");
            return;
        }
        if self.has_restorable_changes() {
            restore_now(self, "程序退出，已自动恢复");
        }
    }
}

/// 后台工作循环（阻塞调用线程）。
///
/// 只做一件事：等命令、执行命令。没有定时检测，空闲时每 200 ms 醒一次
/// 确认没有新命令就继续睡。
pub fn run_worker(shared: Arc<Shared>) -> ! {
    logging::info("后台工作线程已启动");

    loop {
        if shared.quit_requested() {
            std::process::exit(0);
        }

        match shared.take_command() {
            Some(Command::Start) => {
                optimize_now(&shared);
            }
            Some(Command::Stop) => {
                restore_now(&shared, "已手动停止优化，可还原项已恢复");
            }
            None => shared.wait(TICK),
        }
    }
}

/// 立即执行一轮优化（当前启用的所有项）。
pub fn optimize_now(shared: &Shared) -> Vec<ItemResult> {
    if shared.quit_requested() {
        return Vec::new();
    }
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
    }

    let (ids, notify, target) = match shared.config.lock() {
        Ok(cfg) => (
            cfg.enabled_items.clone(),
            cfg.notify_on_start,
            resolve_target(&cfg),
        ),
        Err(_) => {
            shared.set_phase(Phase::Idle);
            return Vec::new();
        }
    };

    if ids.is_empty() {
        shared.set_phase(Phase::Idle);
        return Vec::new();
    }

    logging::info(&format!("开始优化，共 {} 项", ids.len()));
    let results = {
        let mut snap = match shared.snapshot.lock() {
            Ok(s) => s,
            Err(_) => {
                shared.set_phase(Phase::Idle);
                return Vec::new();
            }
        };
        // 每次全量优化前重置快照，避免同一 pid 重复累积。但「本程序关闭、
        // 尚未还原」的程序必须带过来：连续两轮优化之间没有经过 restore，
        // 清掉它等于把上一轮关掉的 Wallpaper Engine 永久留在关闭状态。
        snap.reset_keeping_pending_programs();
        engine::run_items(&ids, target.as_deref(), &mut snap)
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

/// 从配置里取出「要提升优先级的进程」，并再校验一次。
///
/// 配置可能被手工改过（或者用户把某个进程加进了排除清单却忘了清空
/// `game_process`），所以这里不能直接信赖字段内容：命中排除清单就视为
/// 没有目标，只做后台降级。
fn resolve_target(cfg: &Config) -> Option<String> {
    let remembered = cfg.game_process.trim();
    if remembered.is_empty() {
        return None;
    }
    match crate::focus::check_target(cfg, remembered) {
        Ok(name) => Some(name),
        Err(why) => {
            logging::warn(&format!(
                "配置里的 game_process「{remembered}」不可用：{}，本轮只降级后台进程",
                why.reason()
            ));
            None
        }
    }
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
    // 逐项消息只在真的动了什么时才有；补一行说明"是谁触发的还原"，
    // 否则日志里看不出还原发生过（例如只跑了 dns 这种无可还原项时）。
    logging::info(&format!(
        "停止优化：{reason}（还原 {} 项改动）",
        messages.len()
    ));

    let notify = shared.config.lock().map(|c| c.notify_on_stop).unwrap_or(true);

    if let Ok(mut st) = shared.status.lock() {
        st.phase = Phase::Idle;
        st.results.clear();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 「开始」与「还原」是同一个开关的两个方向：待机时给 Start，
    /// 已优化时给 Stop。菜单文案、灰化与快捷键行为都只认这一个判定。
    #[test]
    fn toggle_action_follows_phase() {
        let shared = Shared::new(Config::default());
        assert_eq!(shared.toggle_action(), ToggleAction::Start);
        assert_eq!(shared.toggle_action().label(), "立即开始优化");

        set_phase_for_test(&shared, Phase::Optimizing);
        assert_eq!(shared.toggle_action(), ToggleAction::Busy);
        assert!(!shared.toggle_action().is_available(), "优化中不可操作");

        set_phase_for_test(&shared, Phase::Optimized);
        assert_eq!(shared.toggle_action(), ToggleAction::Stop);
        assert_eq!(shared.toggle_action().label(), "停止优化并还原");
    }

    /// 总开关关闭时两个方向都不可用。
    #[test]
    fn toggle_action_disabled_when_master_off() {
        let cfg = Config {
            enabled: false,
            ..Default::default()
        };
        let shared = Shared::new(cfg);
        assert_eq!(shared.toggle_action(), ToggleAction::Disabled);
        assert!(!shared.toggle_action().is_available());

        // 即使残留了"已优化"状态，关闭总开关也不该让还原变成可点
        set_phase_for_test(&shared, Phase::Optimized);
        assert_eq!(shared.toggle_action(), ToggleAction::Disabled);
    }

    /// 判断「是否还有东西要还原」必须看快照，不能看 phase。
    ///
    /// 优化跑到一半被关掉时 phase 还是「优化中」（甚至待机），但优先级、
    /// 服务、Wallpaper Engine 已经动过了；按 phase 判断会漏还原。
    #[test]
    fn restorable_changes_are_detected_by_snapshot_not_phase() {
        let shared = Shared::new(Config::default());
        assert!(!shared.has_restorable_changes());

        set_phase_for_test(&shared, Phase::Idle);
        if let Ok(mut snap) = shared.snapshot.lock() {
            snap.stopped_services.push("WSearch".into());
        }
        assert!(
            shared.has_restorable_changes(),
            "phase 是待机但快照非空，仍然要还原"
        );
    }

    /// 快照只记录了「关闭的程序」时也算有东西要还原。
    #[test]
    fn killed_program_alone_counts_as_restorable() {
        let shared = Shared::new(Config::default());
        if let Ok(mut snap) = shared.snapshot.lock() {
            snap.killed_programs.push(r"D:\WE\wallpaper64.exe".into());
        }
        assert!(shared.has_restorable_changes());
    }

    /// 优雅退出不得凭空造出还原：空快照时 shutdown 只置退出标志。
    #[test]
    fn shutdown_without_changes_sends_no_notification() {
        let shared = Shared::new(Config::default());
        shared.shutdown();
        assert!(shared.quit_requested());
        assert!(shared.drain_notifications().is_empty(), "没有改动就不该弹还原通知");
    }

    /// 判定与投递是同一个函数：返回的命令与判定必须一致，不可用的状态
    /// 不得投递任何命令。
    #[test]
    fn request_toggle_matches_action() {
        let shared = Shared::new(Config::default());
        let (action, command) = shared.request_toggle();
        assert_eq!(action, ToggleAction::Start);
        assert_eq!(command, Some(Command::Start));

        set_phase_for_test(&shared, Phase::Optimized);
        let (action, command) = shared.request_toggle();
        assert_eq!(action, ToggleAction::Stop);
        assert_eq!(command, Some(Command::Stop));

        set_phase_for_test(&shared, Phase::Optimizing);
        let (action, command) = shared.request_toggle();
        assert_eq!(action, ToggleAction::Busy);
        assert_eq!(command, None, "优化中不得投递任何命令");

        if let Ok(mut c) = shared.config.lock() {
            c.enabled = false;
        }
        set_phase_for_test(&shared, Phase::Idle);
        let (action, command) = shared.request_toggle();
        assert_eq!(action, ToggleAction::Disabled);
        assert_eq!(command, None, "总开关关闭时不得投递任何命令");
    }

    /// 连续切换两个方向：Start → Stop，每次都有对应的命令，不会只剩 start。
    #[test]
    fn toggle_alternates_between_start_and_stop() {
        let shared = Shared::new(Config::default());

        let (first, _) = shared.request_toggle();
        assert_eq!(first, ToggleAction::Start);

        // 模拟后台跑完
        set_phase_for_test(&shared, Phase::Optimized);

        let (second, _) = shared.request_toggle();
        assert_eq!(second, ToggleAction::Stop);

        // 模拟还原完成
        set_phase_for_test(&shared, Phase::Idle);

        let (third, _) = shared.request_toggle();
        assert_eq!(third, ToggleAction::Start, "还原后应能再次开始");
    }

    /// 总开关关闭时不接受优化请求，状态也不会卡在「优化中」。
    #[test]
    fn optimize_request_ignored_when_master_off() {
        let cfg = Config {
            enabled: false,
            ..Default::default()
        };
        let shared = Shared::new(cfg);
        let results = optimize_now(&shared);
        assert!(results.is_empty());
        assert_eq!(shared.phase(), Phase::Idle);
    }

    fn set_phase_for_test(shared: &Shared, phase: Phase) {
        if let Ok(mut st) = shared.status.lock() {
            st.phase = phase;
        }
    }
}
