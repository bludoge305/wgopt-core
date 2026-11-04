//! 优化引擎：8 项优化的执行逻辑与还原。
//!
//! 与原版逐行对齐，只把 `status: String` 改为 `Status` 枚举。

use crate::item::{ItemResult, Status};
use crate::snapshot::Snapshot;
use crate::whitelist;
use crate::winapi;

// ---------------------------------------------------------------- 各项优化

/// 1. 调整进程优先级：游戏进程升到「高」，白名单后台进程降到「低于正常」。
pub fn optimize_priority(target_game: Option<&str>, snapshot: &mut Snapshot) -> ItemResult {
    let started = std::time::Instant::now();
    // 只需要 pid 与名称，用轻量枚举避免全量句柄查询的开销
    let basic = winapi::list_processes_basic();

    let mut promoted = 0usize;
    let mut demoted = 0usize;

    for (pid, name) in &basic {
        let lower = name.to_ascii_lowercase();
        let pid = *pid;
        if whitelist::is_protected(name) {
            continue;
        }

        let is_game = match target_game {
            Some(g) => lower == g.to_ascii_lowercase(),
            None => whitelist::is_known_game(name),
        };

        if is_game {
            // set_process_priority 内部会返回原优先级，无需事先查询
            if let Ok(before) = winapi::set_process_priority(pid, "高") {
                if before != "高" {
                    snapshot.promoted.push((pid, name.clone()));
                    promoted += 1;
                }
            }
            continue;
        }

        if whitelist::in_background_whitelist(name) {
            if let Ok(before) = winapi::set_process_priority(pid, "低于正常") {
                if before != "低于正常" && before != "低" {
                    snapshot.demoted.push((pid, name.clone(), before));
                    demoted += 1;
                }
            }
        }
    }

    let detail = if promoted == 0 && demoted == 0 {
        "未发现需要调整的进程".to_string()
    } else if promoted == 0 {
        format!("{demoted} 个后台进程已降级")
    } else if demoted == 0 {
        format!("{promoted} 个游戏进程已提升为高优先级")
    } else {
        format!("{promoted} 个游戏进程提升为高，{demoted} 个后台进程降级")
    };

    ItemResult {
        id: "priority".into(),
        status: Status::Done,
        detail,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

/// 2. 结束白名单内的后台进程。
pub fn optimize_background(game_name: Option<&str>) -> ItemResult {
    let started = std::time::Instant::now();
    let mut killed: Vec<String> = Vec::new();
    let mut denied: Vec<String> = Vec::new();

    for p in winapi::list_processes() {
        if whitelist::is_protected(&p.name) {
            continue;
        }
        // 不结束游戏自身
        if let Some(g) = game_name {
            if p.name.eq_ignore_ascii_case(g) {
                continue;
            }
        }
        if !whitelist::in_background_whitelist(&p.name) {
            continue;
        }
        match winapi::terminate_process(p.pid) {
            Ok(_) => {
                if !killed.iter().any(|k| k.eq_ignore_ascii_case(&p.name)) {
                    killed.push(p.name.clone());
                }
            }
            Err(_) => {
                if !denied.iter().any(|k| k.eq_ignore_ascii_case(&p.name)) {
                    denied.push(p.name.clone());
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
    let targets = [
        "wallpaper32.exe",
        "wallpaper64.exe",
        "ui32.exe",
        "webwallpaper32.exe",
        "webwallpaper64.exe",
    ];
    let mut killed = Vec::new();

    for p in winapi::list_processes() {
        if !targets.iter().any(|t| t.eq_ignore_ascii_case(&p.name)) {
            continue;
        }
        // 先取路径再结束：进程没了就查不到路径，还原时无从启动
        let path = winapi::process_image_path(p.pid);
        if winapi::terminate_process(p.pid).is_ok() {
            if !killed.iter().any(|k: &String| k.eq_ignore_ascii_case(&p.name)) {
                killed.push(p.name.clone());
            }
            if !path.is_empty()
                && !snapshot
                    .killed_programs
                    .iter()
                    .any(|q| q.eq_ignore_ascii_case(&path))
            {
                snapshot.killed_programs.push(path);
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
pub fn run_item(id: &str, game: Option<&str>, snapshot: &mut Snapshot) -> ItemResult {
    match id {
        "priority" => optimize_priority(game, snapshot),
        "background" => optimize_background(game),
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
pub fn run_items(ids: &[String], game: Option<&str>, snapshot: &mut Snapshot) -> Vec<ItemResult> {
    ids.iter()
        .map(|id| run_item(id, game, snapshot))
        .collect()
}

/// 恢复快照：还原进程优先级、重启服务。
pub fn restore(snapshot: &Snapshot) -> Vec<String> {
    let mut messages = Vec::new();

    for (pid, name, original) in &snapshot.demoted {
        if winapi::set_process_priority(*pid, original).is_ok() {
            messages.push(format!("已恢复 {name} 优先级为{original}"));
        }
    }

    for (_pid, name) in &snapshot.promoted {
        messages.push(format!("已取消 {name} 的高优先级"));
    }

    for service in &snapshot.stopped_services {
        match winapi::start_service(service) {
            Ok(_) => messages.push(format!("已恢复服务 {service}")),
            Err(e) => messages.push(format!("恢复服务 {service} 失败：{e}")),
        }
    }

    // 重新拉起被关闭的程序（Wallpaper Engine）。
    // 用户可能已经自己手动开回来了，这种情况不重复启动。
    for path in &snapshot.killed_programs {
        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());

        if winapi::list_processes_basic()
            .iter()
            .any(|(_, running)| running.eq_ignore_ascii_case(&name))
        {
            messages.push(format!("{name} 已在运行，无需重启"));
            continue;
        }

        match winapi::spawn_process(path) {
            Ok(_) => messages.push(format!("已重新启动 {name}")),
            Err(e) => messages.push(format!("重新启动 {name} 失败：{e}")),
        }
    }

    messages
}
