//! 优化引擎：8 项优化的执行逻辑与还原。
//!
//! 与原版逐行对齐，只把 `status: String` 改为 `Status` 枚举。

use crate::item::{ItemResult, Status};
use crate::logging;
use crate::snapshot::Snapshot;
use crate::whitelist;
use crate::winapi;

// ---------------------------------------------------------------- Wallpaper Engine

/// Wallpaper Engine 相关进程：主进程、UI 与 Web 渲染子进程。
///
/// `background` 与 `wallpaper` 两项都可能结束它们，所以名单放在这里共用。
const WALLPAPER_PROCESSES: &[&str] = &[
    "wallpaper32.exe",
    "wallpaper64.exe",
    "ui32.exe",
    "webwallpaper32.exe",
    "webwallpaper64.exe",
];

/// Wallpaper Engine 主进程：负责渲染桌面，并会自己派生 UI 与 Web 子进程。
/// 还原时只需要拉起其中一个。
const WALLPAPER_MAIN_PROCESSES: &[&str] = &["wallpaper32.exe", "wallpaper64.exe"];

fn is_wallpaper_process(name: &str) -> bool {
    WALLPAPER_PROCESSES
        .iter()
        .any(|t| t.eq_ignore_ascii_case(name))
}

fn is_wallpaper_main(name: &str) -> bool {
    WALLPAPER_MAIN_PROCESSES
        .iter()
        .any(|t| t.eq_ignore_ascii_case(name))
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

/// 登记一个「本程序关闭、还原时需要重新拉起」的程序。
///
/// 只记录 Wallpaper Engine：它被关掉会直接把用户的桌面壁纸弄没，
/// 属于必须还原的副作用；其他后台进程按设计不还原。
fn record_killed_program(snapshot: &mut Snapshot, name: &str, path: &str) {
    if !is_wallpaper_process(name) {
        return;
    }
    if path.is_empty() {
        // 取不到路径就没法还原，必须留下痕迹，否则用户只会看到"壁纸没了"
        logging::warn(&format!("{name} 已关闭但取不到可执行文件路径，还原时无法重新启动"));
        return;
    }
    if snapshot
        .killed_programs
        .iter()
        .any(|recorded| recorded.eq_ignore_ascii_case(path))
    {
        return;
    }
    snapshot.killed_programs.push(path.to_string());
}

/// 从关闭记录里挑出还原时要启动的入口程序。
///
/// 一次优化可能关掉主进程、UI 和 Web 子进程，逐个拉起会得到多份
/// UI/渲染进程；只启动一个主进程，其余子进程由它自己派生。记录里没有
/// 主进程时（例如只关掉了 UI）退回第一个记录，保持最小的恢复动作。
fn pick_relaunch_entry(paths: &[String]) -> Option<String> {
    if let Some(main) = paths.iter().find(|p| is_wallpaper_main(&file_name(p))) {
        return Some(main.clone());
    }
    paths.first().cloned()
}

fn any_wallpaper_main_running(running: &[String]) -> bool {
    running
        .iter()
        .any(|name| WALLPAPER_MAIN_PROCESSES.iter().any(|m| name.eq_ignore_ascii_case(m)))
}

/// 是否应跳过启动。
///
/// 目标进程本身已在运行时跳过；此外对 Wallpaper Engine 还要看主进程——
/// 用户可能已经自己开回来了，而且开的可能是另一个位数的主进程（记录里
/// 是 64 位、用户开的是 32 位），那也算已恢复，不能再启动第二份。
/// 其他程序（快照里理论上不该有）只按名字判断，不能因为 WE 恰好在跑
/// 就误报成"已在运行"。
fn skip_relaunch(running: &[String], target: &str) -> bool {
    running
        .iter()
        .any(|name| name.eq_ignore_ascii_case(target))
        || (is_wallpaper_process(target) && any_wallpaper_main_running(running))
}

/// 进程是否出现在进程表里。
///
/// 每次调用都重新枚举一次进程表——`confirm_started` 要的就是「此刻在不在」
/// 的新鲜数据，没法复用上一次的结果。
fn process_seen(name: &str) -> bool {
    let running: Vec<String> = winapi::list_processes_basic()
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    skip_relaunch(&running, name)
}

/// 启动后确认进程确实活下来了，而不是建出来就退出。
///
/// `CreateProcess` 成功只代表进程对象创建成功：缺运行库、Steam 未启动时
/// 程序会立刻退出，但退出前那一小段时间仍能在进程表里查到它。所以不能
/// 一次查到就报成功，要等它出现、再等一小段确认仍然在。
///
/// Wallpaper Engine 允许被启动的那个位数不在、但另一个位数的主进程在跑
/// （32 位启动器可能切到 64 位），对这种情况同样按主进程名单判定。
fn confirm_started(name: &str) -> bool {
    /// 首次出现前的等待：立刻退出的程序走完这段也不会被当成成功
    const SETTLE_BEFORE_CHECK: std::time::Duration = std::time::Duration::from_millis(400);
    const POLL: std::time::Duration = std::time::Duration::from_millis(200);
    /// 出现后再确认一次的间隔，用于排除"刚起来就退出"
    const CONFIRM_AGAIN: std::time::Duration = std::time::Duration::from_millis(600);

    let wallpaper = is_wallpaper_process(name);
    // Wallpaper Engine 启动要读资源、可能经 Steam 拉起，给足 6 秒；
    // 其他程序理论上不该出现在记录里，2 秒足够。
    let polls = if wallpaper { 30 } else { 10 };

    std::thread::sleep(SETTLE_BEFORE_CHECK);

    let mut seen = process_seen(name);
    for _ in 0..polls {
        if seen {
            break;
        }
        std::thread::sleep(POLL);
        seen = process_seen(name);
    }
    if !seen {
        return false;
    }

    std::thread::sleep(CONFIRM_AGAIN);
    process_seen(name)
}

// ---------------------------------------------------------------- 各项优化

/// 1. 调整进程优先级：目标进程升到「高」，白名单后台进程降到「低于正常」。
///
/// `target` 是快捷键触发时识别出的前台进程名（已通过排除清单校验）。
/// 同名进程可能不止一个，全部提升；命中保护名单或不在进程表里时跳过。
pub fn optimize_priority(target: Option<&str>, snapshot: &mut Snapshot) -> ItemResult {
    let started = std::time::Instant::now();
    // 只需要 pid 与名称，用轻量枚举避免全量句柄查询的开销
    let processes = winapi::list_processes_basic();

    let mut promoted = 0usize;
    let mut demoted = 0usize;

    if let Some(game) = target {
        for (pid, name) in &processes {
            if !name.eq_ignore_ascii_case(game) {
                continue;
            }
            if whitelist::is_protected(name) {
                continue;
            }
            // set_process_priority 返回设置前的优先级
            match winapi::set_process_priority(*pid, "高") {
                Ok(before) if before != "高" => {
                    snapshot.promoted.push((*pid, name.clone(), before));
                    promoted += 1;
                }
                _ => {}
            }
        }
    }

    for (pid, name) in &processes {
        if whitelist::is_protected(name) || !whitelist::in_background_whitelist(name) {
            continue;
        }
        match winapi::set_process_priority(*pid, "低于正常") {
            Ok(before) if before != "低于正常" && before != "低" => {
                snapshot.demoted.push((*pid, name.clone(), before));
                demoted += 1;
            }
            _ => {}
        }
    }

    let detail = match (promoted, demoted) {
        (0, 0) => "未发现需要调整的进程".to_string(),
        (0, d) => format!("{d} 个后台进程已降级"),
        (p, 0) => format!("{p} 个进程已提升为高优先级"),
        (p, d) => format!("{p} 个进程提升为高，{d} 个后台进程降级"),
    };

    ItemResult {
        id: "priority".into(),
        status: Status::Done,
        detail,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

/// 2. 结束白名单内的后台进程。
///
/// 白名单里也包含 Wallpaper Engine。它一旦在这里被结束，还原时就再也
/// 找不到它了（进程已消失，查不到路径），所以与第 4 项一样，在结束前
/// 取路径登记进快照。
pub fn optimize_background(snapshot: &mut Snapshot) -> ItemResult {
    let started = std::time::Instant::now();
    let mut killed: Vec<String> = Vec::new();
    let mut denied: Vec<String> = Vec::new();

    // 只需要 pid 与名称：结束进程不需要工作集/句柄数，
    // 用轻量枚举省掉对每个进程的句柄查询
    for (pid, name) in winapi::list_processes_basic() {
        if whitelist::is_protected(&name) {
            continue;
        }
        if !whitelist::in_background_whitelist(&name) {
            continue;
        }

        // 必须在结束之前取路径：进程没了就查不到，还原时无从启动
        let is_wallpaper = is_wallpaper_process(&name);
        let path = if is_wallpaper {
            winapi::process_image_path(pid)
        } else {
            String::new()
        };

        match winapi::terminate_process(pid) {
            Ok(_) => {
                record_killed_program(snapshot, &name, &path);
                if !killed.iter().any(|k| k.eq_ignore_ascii_case(&name)) {
                    killed.push(name.clone());
                }
            }
            Err(_) => {
                if !denied.iter().any(|k| k.eq_ignore_ascii_case(&name)) {
                    denied.push(name.clone());
                }
            }
        }
    }

    let elevated = winapi::is_elevated();

    let (status, detail) = if killed.is_empty() && denied.is_empty() {
        (Status::Skip, "未发现需要关闭的后台进程".to_string())
    } else if killed.is_empty() {
        // 有目标但一个都没成功：区分"权限不足"与"系统保护"
        let list = denied.join("、");
        let reason = if elevated {
            format!("{list} 受系统保护，无法结束")
        } else {
            format!("{list} 需要管理员权限才能结束")
        };
        (Status::Skip, reason)
    } else if denied.is_empty() {
        (
            Status::Done,
            format!("已结束 {} 个进程：{}", killed.len(), killed.join("、")),
        )
    } else {
        (
            Status::Done,
            format!(
                "已结束 {} 个进程：{}；{} 未能结束",
                killed.len(),
                killed.join("、"),
                denied.join("、")
            ),
        )
    };

    ItemResult {
        id: "background".into(),
        status,
        detail,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

/// 3. 释放内存。
pub fn optimize_memory() -> ItemResult {
    let started = std::time::Instant::now();
    let before = winapi::available_physical();
    let (freed, trimmed, _failed) = winapi::trim_all_working_sets();
    let after = winapi::available_physical();

    let detail = if freed > 0 {
        format!(
            "可用内存 {} → {}（释放 {}），整理 {trimmed} 个进程",
            winapi::format_bytes(before),
            winapi::format_bytes(after),
            winapi::format_bytes(freed)
        )
    } else {
        format!("已整理 {trimmed} 个进程的工作集，当前可用 {}", winapi::format_bytes(after))
    };

    ItemResult {
        id: "memory".into(),
        status: Status::Done,
        detail,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

/// 4. 关闭 Wallpaper Engine（独立项，不属于通用后台白名单）。
///
/// 关闭前记录每个进程的可执行文件路径，停止优化时按此重新拉起。
/// 只要原程序能取到路径就视为可还原——Wallpaper Engine 是用户主动
/// 开着才有桌面壁纸的，关掉不还原等于把用户的桌面弄没了。
pub fn optimize_wallpaper(snapshot: &mut Snapshot) -> ItemResult {
    let started = std::time::Instant::now();
    let mut killed = Vec::new();

    // 同样只需要 pid 与名称
    for (pid, name) in winapi::list_processes_basic() {
        if !is_wallpaper_process(&name) {
            continue;
        }
        // 先取路径再结束：进程没了就查不到路径，还原时无从启动
        let path = winapi::process_image_path(pid);
        if winapi::terminate_process(pid).is_ok() {
            record_killed_program(snapshot, &name, &path);
            if !killed.iter().any(|k: &String| k.eq_ignore_ascii_case(&name)) {
                killed.push(name.clone());
            }
        }
    }

    let (status, detail) = if killed.is_empty() {
        (Status::Skip, "未检测到 Wallpaper Engine，已跳过".to_string())
    } else {
        (Status::Done, format!("已关闭：{}", killed.join("、")))
    };

    ItemResult {
        id: "wallpaper".into(),
        status,
        detail,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

/// 5/6. 停止服务（Xbox / Windows Search）。
pub fn optimize_service(
    id: &str,
    service_name: &str,
    label: &str,
    snapshot: &mut Snapshot,
) -> ItemResult {
    let started = std::time::Instant::now();

    match winapi::service_running(service_name) {
        Ok(true) => match winapi::stop_service(service_name) {
            Ok(was_running) => {
                if was_running {
                    snapshot.stopped_services.push(service_name.to_string());
                    ItemResult {
                        id: id.into(),
                        status: Status::Done,
                        detail: format!("{label} 已停止"),
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    }
                } else {
                    ItemResult {
                        id: id.into(),
                        status: Status::Skip,
                        detail: format!("{label} 当前未运行"),
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    }
                }
            }
            Err(e) => ItemResult {
                id: id.into(),
                status: Status::Fail,
                detail: e,
                elapsed_ms: started.elapsed().as_millis() as u64,
            },
        },
        Ok(false) => ItemResult {
            id: id.into(),
            status: Status::Skip,
            detail: format!("{label} 原本就未运行"),
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
        Err(e) => ItemResult {
            id: id.into(),
            status: Status::Fail,
            detail: e,
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
    }
}

/// 7. 清理 DNS。
pub fn optimize_dns() -> ItemResult {
    let started = std::time::Instant::now();
    match winapi::flush_dns() {
        Ok(_) => ItemResult {
            id: "dns".into(),
            status: Status::Done,
            detail: "DNS 解析缓存已刷新".into(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
        Err(e) => ItemResult {
            id: "dns".into(),
            status: Status::Fail,
            detail: e,
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
    }
}

/// 8. 清理剪贴板。
pub fn optimize_clipboard() -> ItemResult {
    let started = std::time::Instant::now();
    match winapi::clear_clipboard() {
        Ok(_) => ItemResult {
            id: "clipboard".into(),
            status: Status::Done,
            detail: "粘贴板已清空".into(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
        Err(e) => ItemResult {
            id: "clipboard".into(),
            status: Status::Fail,
            detail: e,
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
    }
}

// ---------------------------------------------------------------- 调度

/// 按 id 执行单个优化项。
pub fn run_item(id: &str, target: Option<&str>, snapshot: &mut Snapshot) -> ItemResult {
    match id {
        "priority" => optimize_priority(target, snapshot),
        "background" => optimize_background(snapshot),
        "memory" => optimize_memory(),
        "wallpaper" => optimize_wallpaper(snapshot),
        "xbox" => optimize_service("xbox", "XblAuthManager", "Xbox Live 身份验证管理器", snapshot),
        "search" => optimize_service("search", "WSearch", "Windows Search", snapshot),
        "dns" => optimize_dns(),
        "clipboard" => optimize_clipboard(),
        other => ItemResult {
            id: other.into(),
            status: Status::Fail,
            detail: format!("未知的优化项：{other}"),
            elapsed_ms: 0,
        },
    }
}

/// 按给定顺序逐项执行优化。
pub fn run_items(ids: &[String], target: Option<&str>, snapshot: &mut Snapshot) -> Vec<ItemResult> {
    ids.iter()
        .map(|id| run_item(id, target, snapshot))
        .collect()
}

/// 恢复快照：还原进程优先级、重启服务。
pub fn restore(snapshot: &Snapshot) -> Vec<String> {
    let mut messages = Vec::new();

    // 进程可能在优化期间已经退出（游戏关掉是常态），此时 OpenProcess
    // 会失败。这不是错误，但不能一声不响——否则用户只看到"已还原"
    // 却不知道有一项没恢复。
    for (pid, name, original) in &snapshot.promoted {
        match winapi::set_process_priority(*pid, original) {
            Ok(_) => messages.push(format!("已恢复 {name} 优先级为{original}")),
            Err(e) => messages.push(format!("恢复 {name} 优先级失败：{e}")),
        }
    }

    for (pid, name, original) in &snapshot.demoted {
        match winapi::set_process_priority(*pid, original) {
            Ok(_) => messages.push(format!("已恢复 {name} 优先级为{original}")),
            Err(e) => messages.push(format!("恢复 {name} 优先级失败：{e}")),
        }
    }

    for service in &snapshot.stopped_services {
        match winapi::start_service(service) {
            Ok(_) => messages.push(format!("已恢复服务 {service}")),
            Err(e) => messages.push(format!("恢复服务 {service} 失败：{e}")),
        }
    }

    // 重新拉起被关闭的程序（Wallpaper Engine）。
    //
    // 只处理本程序确实关掉并对路径的进程：快照为空说明优化时它原本就没运行，
    // 不做任何事。运行状态按进程名判断，覆盖用户手动开回来（可能开的是另一
    // 个位数的主进程）与启动失败两种情况。
    let running: Vec<String> = winapi::list_processes_basic()
        .into_iter()
        .map(|(_, name)| name)
        .collect();

    if let Some(entry) = pick_relaunch_entry(&snapshot.killed_programs) {
        let name = file_name(&entry);
        if skip_relaunch(&running, &name) {
            messages.push(format!("{name} 已在运行，无需重启"));
        } else {
            match winapi::spawn_process(&entry) {
                Ok(_) if confirm_started(&name) => {
                    messages.push(format!("已重新启动 {name}"))
                }
                Ok(_) => messages.push(format!(
                    "重新启动 {name} 失败：进程启动后立即退出（可能是运行库缺失或 Steam 未启动）"
                )),
                Err(e) => messages.push(format!("重新启动 {name} 失败：{e}")),
            }
        }
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallpaper_process_names_cover_main_and_helpers() {
        for name in ["wallpaper64.exe", "WALLPAPER32.EXE", "ui32.exe", "webwallpaper64.exe"] {
            assert!(is_wallpaper_process(name), "{name} 应被识别为 Wallpaper Engine");
        }
        assert!(!is_wallpaper_process("wallpaper_engine_helper.exe"));
        assert!(!is_wallpaper_process("explorer.exe"));
    }

    /// 后台项结束 Wallpaper Engine 时也要登记路径，
    /// 否则随后的 wallpaper 项只看得到"未检测到"（日志里的真实故障）。
    #[test]
    fn background_item_records_wallpaper_path() {
        let mut snap = Snapshot::default();
        record_killed_program(
            &mut snap,
            "wallpaper64.exe",
            r"D:\Steam\steamapps\common\wallpaper_engine\wallpaper64.exe",
        );
        assert_eq!(snap.killed_programs.len(), 1);
    }

    /// 同一个路径被两项各记一次（background 与 wallpaper 都命中）时只留一条。
    #[test]
    fn duplicate_paths_are_recorded_once() {
        let mut snap = Snapshot::default();
        let path = r"D:\Steam\wallpaper_engine\wallpaper64.exe";
        record_killed_program(&mut snap, "wallpaper64.exe", path);
        record_killed_program(&mut snap, "wallpaper64.exe", path);
        record_killed_program(&mut snap, "wallpaper64.exe", &path.to_uppercase());
        assert_eq!(snap.killed_programs.len(), 1, "大小写不同也算同一路径");
    }

    /// 取不到路径时不能记空串：spawn_process 会失败，还会让快照误判为有可还原项。
    #[test]
    fn empty_path_is_not_recorded() {
        let mut snap = Snapshot::default();
        record_killed_program(&mut snap, "wallpaper64.exe", "");
        assert!(snap.killed_programs.is_empty());
        assert!(snap.is_empty());
    }

    /// 普通后台进程（OneDrive 等）按设计不还原，不能进关闭记录。
    #[test]
    fn non_wallpaper_processes_are_not_recorded() {
        let mut snap = Snapshot::default();
        record_killed_program(&mut snap, "onedrive.exe", r"C:\OneDrive\OneDrive.exe");
        record_killed_program(&mut snap, "thunder.exe", r"C:\Thunder\Thunder.exe");
        assert!(snap.killed_programs.is_empty());
    }

    /// 一次优化可能关掉主进程、ui32 与 webwallpaper，还原只拉起一个主进程。
    #[test]
    fn relaunch_picks_single_main_process() {
        let paths = vec![
            r"D:\WE\ui32.exe".to_string(),
            r"D:\WE\webwallpaper64.exe".to_string(),
            r"D:\WE\wallpaper64.exe".to_string(),
        ];
        assert_eq!(
            pick_relaunch_entry(&paths).as_deref(),
            Some(r"D:\WE\wallpaper64.exe")
        );

        // 只有 32 位主进程时用它
        let paths = vec![
            r"D:\WE\ui32.exe".to_string(),
            r"D:\WE\wallpaper32.exe".to_string(),
        ];
        assert_eq!(
            pick_relaunch_entry(&paths).as_deref(),
            Some(r"D:\WE\wallpaper32.exe")
        );
    }

    /// 记录里没有主进程（只关掉了 UI / Web 渲染）时退回第一条，不空转。
    #[test]
    fn relaunch_falls_back_to_first_record() {
        let paths = vec![r"D:\WE\ui32.exe".to_string()];
        assert_eq!(pick_relaunch_entry(&paths).as_deref(), Some(r"D:\WE\ui32.exe"));
        assert_eq!(pick_relaunch_entry(&[]), None);
    }

    /// 用户已手动开回来：不能再启动第二份。
    /// 他可能开的是另一个位数的主进程，所以按主进程名单整体判断。
    #[test]
    fn skip_relaunch_when_any_main_is_running() {
        let running = vec!["wallpaper32.exe".to_string(), "explorer.exe".to_string()];
        assert!(skip_relaunch(&running, "wallpaper64.exe"));
        assert!(skip_relaunch(&running, "wallpaper32.exe"));
        // 只有子进程在跑不算恢复完成，仍要拉起主进程
        let only_helper = vec!["ui32.exe".to_string()];
        assert!(!skip_relaunch(&only_helper, "wallpaper64.exe"));
    }

    /// WE 恰好在跑不能成为其他程序的跳过理由，
    /// 否则路径失效等真实失败会被误报成"已在运行"。
    #[test]
    fn other_programs_are_not_skipped_by_running_wallpaper() {
        let running = vec!["wallpaper64.exe".to_string()];
        assert!(!skip_relaunch(&running, "no_such_proc.exe"));
        assert!(!skip_relaunch(&running, "cmd.exe"));
    }

    /// 原本未运行：快照里没有记录，还原不得凭空启动任何东西。
    #[test]
    fn empty_snapshot_restore_starts_nothing() {
        let messages = restore(&Snapshot::default());
        assert!(messages.is_empty(), "空快照不该产生任何消息：{messages:?}");
    }

    /// 路径失效要给出可读失败说明，而不是静默成功。
    #[test]
    fn missing_executable_reports_failure() {
        let snap = Snapshot {
            killed_programs: vec![r"C:\NoSuchDir\no_such_proc.exe".to_string()],
            ..Default::default()
        };
        let messages = restore(&snap);
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(messages[0].contains("失败"), "{}", messages[0]);
        assert!(messages[0].contains("no_such_proc.exe"), "{}", messages[0]);
    }

    /// 已在运行时还原不应调用 spawn（重复启动检查要在启动之前生效）。
    #[test]
    fn running_process_skips_spawn() {
        // 用当前进程名构造快照，保证它一定"正在运行"
        let me = std::env::current_exe().unwrap();
        let name = file_name(&me.to_string_lossy());
        let running = vec![name.clone()];
        assert!(skip_relaunch(&running, &name));
    }
}
