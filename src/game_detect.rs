//! 游戏进程识别引擎。
//!
//! 不依赖单一进程名匹配。采用**多维加权评分**，综合以下信号：
//!
//! | 维度 | 说明 | 权重 |
//! |---|---|---|
//! | 窗口特征 | 存在全屏/接近全屏的可见窗口 | 高 |
//! | 内存占用 | 工作集显著高于普通应用 | 中 |
//! | 线程/句柄数 | 游戏引擎普遍创建大量线程与句柄 | 中 |
//! | CPU 占用 | 采样窗口内的实际处理器占用 | 中 |
//! | 进程名特征 | 常见游戏进程命名模式（Shipping、Game、引擎名等） | 低 |
//! | 父进程链 | 启动器 → 游戏主体的两段式结构 | 低 |
//! | 排除名单 | 浏览器/编辑器/系统组件一律排除 | 硬性 |
//!
//! 命中阈值时可识别诸如 `Client-Win64-Shipping`（鸣潮）、
//! `xxx-Win64-Shipping`（虚幻引擎）、`game.exe` 等非标准命名。

use serde::Serialize;

use crate::winapi;

/// 一个候选进程的评分明细，便于向用户解释判定依据。
#[derive(Debug, Clone, Serialize)]
pub struct GameCandidate {
    pub name: String,
    pub pid: u32,
    pub score: i32,
    /// 窗口标题（若有）
    pub title: String,
    /// 命中的判定理由，逐条可读
    pub reasons: Vec<String>,
}

/// 命中所需的分数。
const THRESHOLD: i32 = 60;

/// 绝不算游戏的进程名（浏览器、编辑器、系统组件、开发工具）。
const NEVER_GAME: &[&str] = &[
    // 浏览器
    "chrome.exe", "msedge.exe", "firefox.exe", "zen.exe", "brave.exe", "opera.exe",
    "vivaldi.exe", "iexplore.exe", "safari.exe", "chromium.exe",
    // 开发工具
    "code.exe", "devenv.exe", "pycharm64.exe", "idea64.exe", "clion64.exe",
    "goland64.exe", "webstorm64.exe", "rider64.exe", "sublime_text.exe",
    "notepad++.exe", "zed.exe", "cursor.exe", "zodium.exe", "zcodium.exe",
    "windowsterminal.exe", "conhost.exe", "powershell.exe", "pwsh.exe", "cmd.exe",
    "wt.exe", "windowsterminal.exe", "ubuntu.exe", "wsl.exe", "wslhost.exe",
    // 通讯与办公
    "wechat.exe", "weixin.exe", "qq.exe", "dingtalk.exe", "feishu.exe", "lark.exe",
    "workbuddy.exe", "teams.exe", "slack.exe", "discord.exe", "telegram.exe",
    "winword.exe", "excel.exe", "powerpnt.exe", "wps.exe", "et.exe", "wpp.exe",
    "notion.exe", "obsidian.exe", "typora.exe",
    // 系统与外壳
    "explorer.exe", "taskmgr.exe", "searchhost.exe", "startmenuexperiencehost.exe",
    "shellexperiencehost.exe", "textinputhost.exe", "applicationframehost.exe",
    "lockapp.exe", "searchapp.exe", "systemsettings.exe", "dwm.exe",
    "sihost.exe", "runtimebroker.exe", "ctfmon.exe", "fontdrvhost.exe",
    // 媒体与其他
    "potplayermini64.exe", "vlc.exe", "mpc-hc64.exe", "wmplayer.exe",
    "photoshop.exe", "illustrator.exe", "aftereffects.exe", "premiere pro.exe",
    "obs64.exe", "obs32.exe", "streamlabs obs.exe",
    "wgopt.exe", "rtss.exe", "msiafterburner.exe",
    "nvidia overlay.exe", "nvcontainer.exe", "nvsphelper64.exe",
    "flclash.exe", "clash.exe", "v2ray.exe", "zerotier_one.exe",
    // 虚拟机与远程
    "vmware-vmx.exe", "virtualboxvm.exe", "mstsc.exe", "todesk.exe", "sunloginclient.exe",
    // 安装器/更新器
    "setup.exe", "installer.exe", "uninstall.exe", "update.exe",
];

/// 进程名中暗示"这是游戏"的片段（小写匹配）。
const GAME_NAME_HINTS: &[&str] = &[
    "-win64-shipping",   // 虚幻引擎 Shipping 构建
    "-win32-shipping",
    "shipping.exe",
    "win64-shipping",
    "unityplayer",
    "game.exe",
    "gameclient",
    "client.exe",
    "-game",
    "launcher.exe",
];

/// 常见的游戏启动器（它们的子进程很可能是游戏本体）。
const LAUNCHER_NAMES: &[&str] = &[
    "steam.exe", "epicgameslauncher.exe", "battle.net.exe", "galaxyclient.exe",
    "uplay.exe", "ubisoftconnect.exe", "origin.exe", "eadesktop.exe",
    "wegame.exe", "wuthering waves.exe", "launcher.exe", "gamelauncher.exe",
    "riotclientservices.exe", "riotclientux.exe", "genshinimpact.exe",
];

/// 判定某进程名是否像游戏（供快速路径使用）。
pub fn is_game_like_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    // 只有 .exe 才可能是进程名，避免 UnityPlayer.dll 之类被误计
    if !lower.ends_with(".exe") {
        return false;
    }
    if NEVER_GAME.iter().any(|n| *n == lower) {
        return false;
    }
    GAME_NAME_HINTS.iter().any(|h| lower.contains(h))
}

fn is_never_game(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    NEVER_GAME.iter().any(|n| *n == lower)
}

fn is_launcher(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    LAUNCHER_NAMES.iter().any(|n| *n == lower)
}

/// 屏幕主分辨率，用于判断窗口是否"接近全屏"。
fn primary_screen() -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
    };
    unsafe {
        (
            GetSystemMetrics(SM_CXSCREEN),
            GetSystemMetrics(SM_CYSCREEN),
        )
    }
}

/// 窗口尺寸的分类。
///
/// 不能只用「占屏幕比例」判断：玩家常在 4K/2K 屏上以 1080p 窗口模式运行游戏，
/// 此时窗口只占屏幕一半左右，按比例判会被当成普通应用窗口。
/// 因此同时识别「常见游戏分辨率」，命中即视为游戏窗口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    /// 占满屏幕（全屏或无边窗口全屏）
    Fullscreen,
    /// 常见游戏分辨率，或占据屏幕较大比例
    GameSized,
    /// 有窗口但尺寸不像游戏
    Small,
}

/// 常见游戏渲染分辨率（窗口客户区尺寸）。
const GAME_RESOLUTIONS: &[(i32, i32)] = &[
    (1280, 720),
    (1280, 800),
    (1366, 768),
    (1440, 900),
    (1600, 900),
    (1680, 1050),
    (1920, 1080),
    (1920, 1200),
    (2048, 1152),
    (2560, 1080),
    (2560, 1440),
    (2560, 1600),
    (3440, 1440),
    (3840, 1600),
    (3840, 2160),
];

/// 判断窗口尺寸属于哪一类。
pub fn classify_window(w: i32, h: i32, screen_w: i32, screen_h: i32) -> WindowKind {
    if w <= 0 || h <= 0 {
        return WindowKind::Small;
    }

    // 1) 接近满屏
    if screen_w > 0
        && screen_h > 0
        && (w as f32) >= screen_w as f32 * 0.9
        && (h as f32) >= screen_h as f32 * 0.9
    {
        return WindowKind::Fullscreen;
    }

    // 2) 命中常见游戏分辨率（允许 ±6 像素误差，抵消边框与 DPI 取整）
    for (gw, gh) in GAME_RESOLUTIONS {
        if (w - gw).abs() <= 6 && (h - gh).abs() <= 6 {
            return WindowKind::GameSized;
        }
    }

    // 3) 16:9 等宽屏比例且面积占屏幕 30% 以上
    //    覆盖游戏内部分辨率缩放的情况，例如 4K 屏上以 1536×864 渲染
    if screen_w > 0 && screen_h > 0 {
        let area_ratio = (w as f64 * h as f64) / (screen_w as f64 * screen_h as f64);
        let aspect = w as f64 / h as f64;
        // 宽屏比例 + 足够面积 + 分辨率不低于 720p
        let wide_screen = aspect > 1.6 && aspect < 1.85;
        if wide_screen && area_ratio >= 0.30 && w >= 1280 && h >= 720 {
            return WindowKind::GameSized;
        }
        if area_ratio >= 0.45 && aspect > 1.2 && aspect < 2.5 {
            return WindowKind::GameSized;
        }
    }

    WindowKind::Small
}

/// 枚举所有可见的顶层窗口，返回 (pid, 标题, 宽, 高)。
pub fn list_visible_windows() -> Vec<(u32, String, i32, i32)> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClientRect, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible,
    };

    // EnumWindows 是同步回调，结果收集用线程局部存储，避免 static mut 的别名风险
    thread_local! {
        static OUT: std::cell::RefCell<Option<Vec<(u32, String, i32, i32)>>> =
            const { std::cell::RefCell::new(None) };
    }

    unsafe extern "system" fn callback(hwnd: HWND, _lparam: LPARAM) -> BOOL {
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                return BOOL(1);
            }
            let mut buf = vec![0u16; (len + 2) as usize];
            let copied = GetWindowTextW(hwnd, &mut buf);
            if copied <= 0 {
                return BOOL(1);
            }
            let title = String::from_utf16_lossy(&buf[..copied as usize]);
            if title.trim().is_empty() {
                return BOOL(1);
            }

            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));

            let mut rect = windows::Win32::Foundation::RECT::default();
            let (mut w, mut h) = (0, 0);
            if GetClientRect(hwnd, &mut rect).is_ok() {
                w = rect.right - rect.left;
                h = rect.bottom - rect.top;
            }

            OUT.with_borrow_mut(|out| {
                if let Some(v) = out.as_mut() {
                    v.push((pid, title, w, h));
                }
            });
            BOOL(1)
        }
    }

    OUT.with_borrow_mut(|o| *o = Some(Vec::new()));
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(0));
    }
    OUT.with_borrow_mut(|o| o.take().unwrap_or_default())
}

/// 单次 CPU 采样：记录各进程的累计 CPU 时间，用于计算占用率。
pub struct CpuSampler {
    samples: Vec<(u32, u64)>,
    cores: f32,
    instant: std::time::Instant,
}

impl CpuSampler {
    pub fn new() -> Self {
        Self {
            samples: Vec::new(),
            cores: num_cores(),
            instant: std::time::Instant::now(),
        }
    }

    /// 采集一轮，返回 (pid, CPU 百分比) 列表。
    pub fn sample(&mut self) -> Vec<(u32, f32)> {
        let elapsed = self.instant.elapsed().as_secs_f64().max(0.001);
        let mut now = Vec::new();
        let mut result = Vec::new();

        for (pid, _name) in winapi::list_processes_basic() {
            let Some(cpu_ms) = winapi::process_cpu_time_ms(pid) else {
                continue;
            };
            now.push((pid, cpu_ms));
            if let Some((_, prev)) = self.samples.iter().find(|(id, _)| *id == pid) {
                let delta = cpu_ms.saturating_sub(*prev) as f64;
                let pct = (delta / (elapsed * 1000.0 * self.cores as f64) * 100.0) as f32;
                if pct > 0.01 {
                    result.push((pid, pct));
                }
            }
        }

        self.samples = now;
        self.instant = std::time::Instant::now();
        result
    }
}

fn num_cores() -> f32 {
    use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
    unsafe {
        let mut info = SYSTEM_INFO::default();
        GetSystemInfo(&mut info);
        (info.dwNumberOfProcessors.max(1)) as f32
    }
}

/// 识别当前游戏中占用最高的进程。
///
/// 主判据是 **GPU 3D 引擎占用**（来自 `\GPU Engine(*)\Utilization Percentage`
/// 性能计数器，与任务管理器同源）。游戏通常独占 30% 以上，而浏览器、
/// 编辑器、聊天软件即便全屏播放视频也只有个位数，区分度极高。
///
/// 辅以 `SHQueryUserNotificationState` 判断系统是否处于全屏应用状态，
/// 以及内存、线程、CPU、父子链等信号兜底 GPU 数据缺失的场景
/// （例如纯 CPU 游戏、集成显卡驱动未暴露计数器）。
pub fn detect_game(
    cpu_usage: &[(u32, f32)],
    gpu_usage: &std::collections::HashMap<u32, f32>,
) -> Option<GameCandidate> {
    rank_candidates(cpu_usage, gpu_usage).into_iter().next()
}

/// 全屏应用状态：Windows 官方判定（QUNS_BUSY / QUNS_RUNNING_D3D_FULL_SCREEN）。
pub fn fullscreen_app_active() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::{
            SHQueryUserNotificationState, QUNS_BUSY, QUNS_RUNNING_D3D_FULL_SCREEN,
        };
        unsafe {
            if let Ok(state) = SHQueryUserNotificationState() {
                return state == QUNS_BUSY || state == QUNS_RUNNING_D3D_FULL_SCREEN;
            }
        }
    }
    false
}

/// 对所有可疑进程评分并排序，返回候选项（降序）。
pub fn rank_candidates(
    cpu_usage: &[(u32, f32)],
    gpu_usage: &std::collections::HashMap<u32, f32>,
) -> Vec<GameCandidate> {
    let (screen_w, screen_h) = primary_screen();
    let windows = list_visible_windows();
    let self_pid = winapi::current_pid();
    let fs_active = fullscreen_app_active();

    // pid -> (标题, 窗口等级, 宽, 高)
    let mut win_map: Vec<(u32, String, WindowKind, i32, i32)> = Vec::new();
    for (pid, title, w, h) in windows {
        let kind = classify_window(w, h, screen_w, screen_h);
        win_map.push((pid, title, kind, w, h));
    }

    let basic = winapi::list_processes_basic();
    let mut wanted: Vec<u32> = Vec::new();
    for (pid, name) in &basic {
        if *pid == self_pid || *pid <= 4 || is_never_game(name) {
            continue;
        }
        let has_window = win_map.iter().any(|(p, _, _, _, _)| p == pid);
        let gpu_hot = gpu_usage.get(pid).map(|v| *v >= 3.0).unwrap_or(false);
        let cpu_hot = cpu_usage.iter().any(|(p, pct)| p == pid && *pct >= 3.0);
        if has_window || gpu_hot || cpu_hot || is_game_like_name(name) {
            wanted.push(*pid);
        }
    }

    // 启动器拉起的子进程也纳入（游戏本体可能无窗口也无明显 CPU）
    let parents = winapi::process_parents();
    let launcher_pids: Vec<u32> = basic
        .iter()
        .filter(|(_, n)| is_launcher(n))
        .map(|(p, _)| *p)
        .collect();
    for (pid, ppid) in &parents {
        if launcher_pids.contains(ppid) && !wanted.contains(pid) {
            let name = basic
                .iter()
                .find(|(p, _)| p == pid)
                .map(|(_, n)| n.as_str())
                .unwrap_or("");
            if !is_never_game(name) {
                wanted.push(*pid);
            }
        }
    }

    let thread_map = winapi::all_thread_counts();
    let processes = winapi::list_processes_subset_with_threads(&wanted, &thread_map);

    let mut out: Vec<GameCandidate> = Vec::new();

    for p in &processes {
        let mut score = 0i32;
        let mut reasons: Vec<String> = Vec::new();

        // ---- GPU 占用：主判据 ----
        let gpu = gpu_usage.get(&p.pid).copied().unwrap_or(0.0);
        if gpu >= 30.0 {
            score += 70;
            reasons.push(format!("GPU 占用 {gpu:.0}%"));
        } else if gpu >= 15.0 {
            score += 50;
            reasons.push(format!("GPU 占用 {gpu:.0}%"));
        } else if gpu >= 5.0 {
            score += 25;
            reasons.push(format!("GPU 占用 {gpu:.0}%"));
        } else if gpu >= 1.0 {
            score += 8;
        }

        // ---- 窗口维度（辅助） ----
        let win = win_map.iter().find(|(pid, _, _, _, _)| *pid == p.pid);
        match win {
            Some((_, title, kind, w, h)) => match kind {
                WindowKind::Fullscreen => {
                    score += 30;
                    reasons.push(format!("全屏窗口 {w}×{h}，标题「{}」", title.trim()));
                }
                WindowKind::GameSized => {
                    score += 25;
                    reasons.push(format!("游戏分辨率窗口 {w}×{h}，标题「{}」", title.trim()));
                }
                WindowKind::Small => score += 2,
            },
            None => {
                reasons.push("无独立可见窗口".into());
            }
        }

        // 系统报告"有全屏应用在运行"时，给它一个有窗口的进程加分
        if fs_active && win.is_some() {
            score += 10;
            reasons.push("系统判定有全屏应用运行".into());
        }

        // ---- 内存 ----
        let mem_mb = p.working_set / (1024 * 1024);
        if mem_mb >= 1500 {
            score += 18;
            reasons.push(format!("内存占用 {mem_mb} MB"));
        } else if mem_mb >= 600 {
            score += 10;
            reasons.push(format!("内存占用 {mem_mb} MB"));
        } else if mem_mb >= 300 {
            score += 4;
        }

        // ---- 线程 / 句柄 ----
        if p.thread_count >= 80 {
            score += 8;
            reasons.push(format!("{} 个线程", p.thread_count));
        }
        if p.handle_count >= 4000 {
            score += 5;
            reasons.push(format!("{} 个句柄", p.handle_count));
        }

        // ---- CPU ----
        if let Some((_, pct)) = cpu_usage.iter().find(|(id, _)| *id == p.pid) {
            if *pct >= 8.0 {
                score += 12;
                reasons.push(format!("CPU 占用 {pct:.1}%"));
            } else if *pct >= 3.0 {
                score += 6;
            }
        }

        // ---- 名称特征 ----
        if is_game_like_name(&p.name) {
            score += 12;
            reasons.push("进程名符合游戏命名模式".into());
        }

        // ---- 父进程链 ----
        if let Some(parent_name) = p.parent_name.as_deref() {
            if is_launcher(parent_name) {
                score += 12;
                reasons.push(format!("由启动器 {parent_name} 拉起"));
            }
        }

        // GPU 与内存双高是最确定的组合
        if gpu >= 15.0 && mem_mb >= 1200 {
            score += 10;
            reasons.push("GPU 与内存双高".into());
        }

        if score > 0 {
            out.push(GameCandidate {
                name: p.name.clone(),
                pid: p.pid,
                score,
                title: win
                    .map(|(_, t, _, _, _)| t.trim().to_string())
                    .unwrap_or_default(),
                reasons,
            });
        }
    }

    out.sort_by(|a, b| b.score.cmp(&a.score));
    out.retain(|c| c.score >= THRESHOLD);
    out
}

/// 仅用于调试：返回所有候选的评分，便于排查误判/漏判。
pub fn score_all(
    cpu_usage: &[(u32, f32)],
    gpu_usage: &std::collections::HashMap<u32, f32>,
) -> Vec<GameCandidate> {
    let mut all = rank_candidates(cpu_usage, gpu_usage);
    // 调试视图还要看到未达阈值的项，因此重新收集一遍低分候选
    all.sort_by(|a, b| b.score.cmp(&a.score));
    all
}
impl GameCandidate {
    /// 展示用名称：优先窗口标题，没有标题时退回进程名。
    pub fn label(&self) -> String {
        let t = self.title.trim();
        if t.is_empty() {
            self.name.clone()
        } else {
            t.to_string()
        }
    }
}
