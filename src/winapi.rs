//! Windows 系统能力封装：进程、服务、内存、剪贴板、DNS。
//!
//! 原样沿用 WGOpt 桌面版的实现，只去掉界面需要的展示字段。
//! 所有对外函数返回 `Result<_, String>`，错误信息为可直接展示的中文。

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{
    EmptyWorkingSet, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
};
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx,
    StartServiceW, SC_HANDLE, SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_QUERY_STATUS,
    SERVICE_RUNNING, SERVICE_START, SERVICE_STATUS, SERVICE_STATUS_PROCESS, SERVICE_STOP,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetPriorityClass, OpenProcess, SetPriorityClass, TerminateProcess,
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, HIGH_PRIORITY_CLASS,
    IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SET_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE, PROCESS_VM_READ,
    REALTIME_PRIORITY_CLASS,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcessToken};

// 直接链接 dnsapi.dll 刷新 DNS 缓存。
// windows crate 0.62 未导出该函数，这里手动声明。
#[link(name = "dnsapi")]
extern "system" {
    fn DnsFlushResolverCache() -> u32;
}

/// 将 Rust 字符串转成以 NUL 结尾的宽字符串。
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 从宽字符缓冲区读取字符串，遇到 NUL 停止。
fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..end]).to_string_lossy().into_owned()
}

// ---------------------------------------------------------------- 权限

/// 判断当前进程是否以管理员身份运行。
pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

// ---------------------------------------------------------------- 进程枚举

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    /// 工作集占用（字节）
    pub working_set: u64,
    /// 线程数量
    pub thread_count: u32,
    /// 句柄数量
    pub handle_count: u32,
    /// 父进程名称（父进程已退出时为空）
    pub parent_name: Option<String>,
}

/// 进程优先级类常量 → 名称
pub fn priority_name(class: u32) -> &'static str {
    match class {
        x if x == IDLE_PRIORITY_CLASS.0 => "低",
        x if x == BELOW_NORMAL_PRIORITY_CLASS.0 => "低于正常",
        x if x == NORMAL_PRIORITY_CLASS.0 => "正常",
        x if x == ABOVE_NORMAL_PRIORITY_CLASS.0 => "高于正常",
        x if x == HIGH_PRIORITY_CLASS.0 => "高",
        x if x == REALTIME_PRIORITY_CLASS.0 => "实时",
        0 => "未知",
        _ => "其他",
    }
}

/// 名称 → 优先级类常量
pub fn priority_class(name: &str) -> Option<windows::Win32::System::Threading::PROCESS_CREATION_FLAGS> {
    Some(match name {
        "低" => IDLE_PRIORITY_CLASS,
        "低于正常" => BELOW_NORMAL_PRIORITY_CLASS,
        "正常" => NORMAL_PRIORITY_CLASS,
        "高于正常" => ABOVE_NORMAL_PRIORITY_CLASS,
        "高" => HIGH_PRIORITY_CLASS,
        _ => return None,
    })
}

/// 枚举所有进程（含内存/线程/句柄统计）。
pub fn list_processes() -> Vec<ProcessInfo> {
    let raw = process_snapshot_raw();
    // 先建 pid -> 名称 映射，供父进程名解析（父进程可能不在列表中）
    let name_map: Vec<(u32, String)> = raw.iter().map(|(p, _, n)| (*p, n.clone())).collect();
    // 线程数一次性统计，避免逐进程重建全系统快照
    let thread_map = all_thread_counts();

    raw.iter()
        .map(|(pid, parent_pid, name)| {
            let (working_set, handle_count) = process_stats(*pid);
            let thread_count = thread_map
                .iter()
                .find(|(p, _)| p == pid)
                .map(|(_, c)| *c)
                .unwrap_or(0);
            ProcessInfo {
                pid: *pid,
                name: name.clone(),
                working_set,
                thread_count,
                handle_count,
                parent_name: name_map
                    .iter()
                    .find(|(p, _)| p == parent_pid)
                    .map(|(_, n)| n.clone()),
            }
        })
        .collect()
}

/// 轻量版进程列表：只取 pid 与名称，不做句柄/内存查询。
/// 用于高频轮询场景，避免反复打开每个进程的句柄。
pub fn list_processes_basic() -> Vec<(u32, String)> {
    process_snapshot_raw()
        .into_iter()
        .map(|(pid, _, name)| (pid, name))
        .collect()
}

/// 所有进程的 (pid, 父 pid) 映射，用于构建父子链。
pub fn process_parents() -> Vec<(u32, u32)> {
    process_snapshot_raw()
        .into_iter()
        .map(|(pid, ppid, _)| (pid, ppid))
        .collect()
}

/// 只统计指定 PID 的详细信息。
///
/// 高频检测时先用 `list_processes_basic` 粗筛，再对少量候选调用本函数。
/// 线程统计通过 `thread_counts_for` 单独完成（它会扫描全系统线程表，
/// 是主要耗时来源），因此调用方应尽量复用其结果。
pub fn list_processes_subset(pids: &[u32]) -> Vec<ProcessInfo> {
    list_processes_subset_with_threads(pids, &thread_counts_for(pids))
}

/// 与 `list_processes_subset` 相同，但线程数由调用方提供，便于复用一次扫描结果。
pub fn list_processes_subset_with_threads(
    pids: &[u32],
    thread_map: &[(u32, u32)],
) -> Vec<ProcessInfo> {
    if pids.is_empty() {
        return Vec::new();
    }
    let raw = process_snapshot_raw();
    let name_map: Vec<(u32, String)> = raw.iter().map(|(p, _, n)| (*p, n.clone())).collect();

    pids.iter()
        .filter_map(|pid| {
            let (_, parent_pid, name) = raw.iter().find(|(p, _, _)| p == pid)?;
            let (working_set, handle_count) = process_stats(*pid);
            let thread_count = thread_map
                .iter()
                .find(|(p, _)| p == pid)
                .map(|(_, c)| *c)
                .unwrap_or(0);
            Some(ProcessInfo {
                pid: *pid,
                name: name.clone(),
                working_set,
                thread_count,
                handle_count,
                parent_name: name_map
                    .iter()
                    .find(|(p, _)| p == parent_pid)
                    .map(|(_, n)| n.clone()),
            })
        })
        .collect()
}

/// 只统计给定 PID 的线程数（仍需一次全表扫描，但结果只保留关心的项）。
fn thread_counts_for(pids: &[u32]) -> Vec<(u32, u32)> {
    use std::collections::{HashMap, HashSet};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };

    let wanted: HashSet<u32> = pids.iter().copied().collect();
    let mut counts: HashMap<u32, u32> = HashMap::new();

    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else {
            return Vec::new();
        };
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        if Thread32First(snapshot, &mut entry).is_ok() {
            loop {
                let owner = entry.th32OwnerProcessID;
                if wanted.contains(&owner) {
                    *counts.entry(owner).or_insert(0) += 1;
                }
                if Thread32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    counts.into_iter().collect()
}

/// 原始进程快照：(pid, 父 pid, 进程名)。
fn process_snapshot_raw() -> Vec<(u32, u32, String)> {
    let mut raw = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return raw;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let pid = entry.th32ProcessID;
                let name = from_wide(&entry.szExeFile);
                if !name.is_empty() && pid != 0 {
                    raw.push((pid, entry.th32ParentProcessID, name));
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    raw
}

/// 一次打开句柄，同时取工作集与句柄数。
///
/// 带反作弊保护的游戏进程常会拒绝 `PROCESS_VM_READ`，此时退回
/// `PROCESS_QUERY_LIMITED_INFORMATION`——工作集与句柄数依然可读，
/// 只有更细的内存细节拿不到。这样能避免把游戏误判成"零内存"进程。
fn process_stats(pid: u32) -> (u64, u32) {
    use windows::Win32::System::Threading::GetProcessHandleCount;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid)
            .or_else(|_| OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid))
            .ok();

        let Some(handle) = handle else {
            return (0, 0);
        };

        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        let ok = K32GetProcessMemoryInfo(
            handle,
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .as_bool();
        let working_set = if ok { counters.WorkingSetSize as u64 } else { 0 };

        let mut handles = 0u32;
        let _ = GetProcessHandleCount(handle, &mut handles);

        let _ = CloseHandle(handle);
        (working_set, handles)
    }
}

/// 一次遍历线程快照，统计所有进程的线程数。
///
/// 逐进程查询会为每个进程重建全系统快照（O(n²)），进程多时开销明显，
/// 因此改为一次性统计。
pub fn all_thread_counts() -> Vec<(u32, u32)> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    let mut result: Vec<(u32, u32)> = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else {
            return result;
        };
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        if Thread32First(snapshot, &mut entry).is_ok() {
            loop {
                let owner = entry.th32OwnerProcessID;
                match result.iter_mut().find(|(p, _)| *p == owner) {
                    Some((_, c)) => *c += 1,
                    None => result.push((owner, 1)),
                }
                if Thread32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    result
}

/// 进程累计 CPU 时间（毫秒），失败返回 None。
pub fn process_cpu_time_ms(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::GetProcessTimes;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel_t = FILETIME::default();
        let mut user_t = FILETIME::default();
        let ok =
            GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel_t, &mut user_t).is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        Some(filetime_to_ms(&kernel_t) + filetime_to_ms(&user_t))
    }
}

fn filetime_to_ms(ft: &windows::Win32::Foundation::FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64) / 10_000
}

/// 设置进程优先级，返回设置前的优先级名。
pub fn set_process_priority(pid: u32, level: &str) -> Result<String, String> {
    let Some(class) = priority_class(level) else {
        return Err(format!("未知的优先级：{level}"));
    };
    unsafe {
        let handle = OpenProcess(PROCESS_SET_INFORMATION, false, pid).map_err(|e| {
            format!("无法打开进程 {pid}（错误码 {}），可能需要管理员权限", e.code().0)
        })?;
        let before = priority_name(GetPriorityClass(handle)).to_string();
        let res = SetPriorityClass(handle, class);
        let _ = CloseHandle(handle);
        res.map_err(|e| format!("设置优先级失败（错误码 {}）", e.code().0))?;
        Ok(before)
    }
}

/// 结束进程。
pub fn terminate_process(pid: u32) -> Result<(), String> {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, false, pid)
            .map_err(|e| format!("无法打开进程 {pid}（错误码 {}）", e.code().0))?;
        let res = TerminateProcess(handle, 1);
        let _ = CloseHandle(handle);
        res.map_err(|e| format!("结束进程失败（错误码 {}）", e.code().0))
    }
}

/// 当前进程 ID。
pub fn current_pid() -> u32 {
    unsafe { GetCurrentProcessId() }
}

/// 取进程可执行文件完整路径，取不到时返回空串。
///
/// 重启被关闭的进程（如 Wallpaper Engine）需要原路径；
/// 受保护进程拒绝 `PROCESS_QUERY_LIMITED_INFORMATION` 时返回空。
pub fn process_image_path(pid: u32) -> String {
    use windows::Win32::System::Threading::{QueryFullProcessImageNameW, PROCESS_NAME_WIN32};

    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if ok && len > 0 {
            from_wide(&buf[..len as usize])
        } else {
            String::new()
        }
    }
}

/// 按路径启动进程，返回新进程 PID（不等待其就绪）。
pub fn spawn_process(exe_path: &str) -> Result<u32, String> {
    use windows::Win32::System::Threading::{
        CreateProcessW, CREATE_NEW_PROCESS_GROUP, PROCESS_INFORMATION, STARTUPINFOW,
    };

    if exe_path.is_empty() {
        return Err("可执行文件路径为空".into());
    }
    if !std::path::Path::new(exe_path).exists() {
        return Err(format!("找不到可执行文件：{exe_path}"));
    }

    let app = wide(exe_path);
    // 工作目录设为程序所在目录：有些程序依赖相对路径读自己的资源
    let workdir = std::path::Path::new(exe_path)
        .parent()
        .map(|p| wide(&p.to_string_lossy()))
        .unwrap_or_else(|| wide(""));

    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut info = PROCESS_INFORMATION::default();

    unsafe {
        CreateProcessW(
            PCWSTR(app.as_ptr()),
            None,
            None,
            None,
            false,
            CREATE_NEW_PROCESS_GROUP,
            None,
            PCWSTR(workdir.as_ptr()),
            &startup,
            &mut info,
        )
        .map_err(|e| format!("启动 {exe_path} 失败（错误码 {}）", e.code().0))?;

        let pid = info.dwProcessId;
        let _ = CloseHandle(info.hProcess);
        let _ = CloseHandle(info.hThread);
        Ok(pid)
    }
}

// ---------------------------------------------------------------- 内存

/// 整理所有可访问进程的工作集，返回 (释放字节数, 成功数, 失败数)。
pub fn trim_all_working_sets() -> (u64, u32, u32) {
    let before = available_physical();
    let mut trimmed = 0u32;
    let mut failed = 0u32;

    for p in list_processes() {
        if p.working_set == 0 {
            continue;
        }
        unsafe {
            let Ok(handle) =
                OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION, false, p.pid)
            else {
                failed += 1;
                continue;
            };
            let ok = EmptyWorkingSet(handle).is_ok();
            let _ = CloseHandle(handle);
            if ok {
                trimmed += 1;
            } else {
                failed += 1;
            }
        }
    }

    // 等待系统回收，再比较可用物理内存
    std::thread::sleep(std::time::Duration::from_millis(600));
    let after = available_physical();
    let freed = after.saturating_sub(before);
    (freed, trimmed, failed)
}

fn memory_status() -> MEMORYSTATUSEX {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe {
        let _ = GlobalMemoryStatusEx(&mut status);
    }
    status
}

/// 当前可用物理内存（字节）。
pub fn available_physical() -> u64 {
    memory_status().ullAvailPhys
}

/// 物理内存总量（字节）。
pub fn total_physical() -> u64 {
    memory_status().ullTotalPhys
}

// ---------------------------------------------------------------- 服务

const SERVICE_CONTROL_STOP_CODE: u32 = 1;

fn open_scm() -> Result<SC_HANDLE, String> {
    unsafe {
        OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)
            .map_err(|e| format!("无法连接服务管理器（错误码 {}），需要管理员权限", e.code().0))
    }
}

fn query_running(svc: SC_HANDLE) -> Result<bool, String> {
    let mut status = SERVICE_STATUS_PROCESS::default();
    let mut needed = 0u32;
    let buf = unsafe {
        std::slice::from_raw_parts_mut(
            &mut status as *mut _ as *mut u8,
            std::mem::size_of::<SERVICE_STATUS_PROCESS>(),
        )
    };
    unsafe {
        QueryServiceStatusEx(svc, SC_STATUS_PROCESS_INFO, Some(buf), &mut needed)
            .map_err(|e| format!("查询服务状态失败（错误码 {}）", e.code().0))?;
    }
    Ok(status.dwCurrentState == SERVICE_RUNNING)
}

/// 查询服务是否运行。
pub fn service_running(name: &str) -> Result<bool, String> {
    unsafe {
        let scm = open_scm()?;
        let wname = wide(name);
        let svc = OpenServiceW(scm, PCWSTR(wname.as_ptr()), SERVICE_QUERY_STATUS);
        let Ok(svc) = svc else {
            let _ = CloseServiceHandle(scm);
            return Err(format!("无法打开服务 {name}，可能不存在"));
        };
        let result = query_running(svc);
        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);
        result
    }
}

/// 停止服务，返回停止前是否在运行。
///
/// 发出停止指令后会等待服务真正进入停止状态，避免调用方紧接着查询
/// 时仍读到"运行中"，也避免恢复时服务正处在停止过程中。
pub fn stop_service(name: &str) -> Result<bool, String> {
    unsafe {
        let scm = open_scm()?;
        let wname = wide(name);
        let svc = OpenServiceW(scm, PCWSTR(wname.as_ptr()), SERVICE_STOP | SERVICE_QUERY_STATUS);
        let Ok(svc) = svc else {
            let _ = CloseServiceHandle(scm);
            return Err(format!("无法打开服务 {name}，可能不存在"));
        };

        let was_running = query_running(svc).unwrap_or(false);
        if !was_running {
            let _ = CloseServiceHandle(svc);
            let _ = CloseServiceHandle(scm);
            return Ok(false);
        }

        let mut status = SERVICE_STATUS::default();
        let res = ControlService(svc, SERVICE_CONTROL_STOP_CODE, &mut status);

        if res.is_err() {
            let code = res.unwrap_err().code().0;
            let _ = CloseServiceHandle(svc);
            let _ = CloseServiceHandle(scm);
            return Err(format!("停止服务 {name} 失败：{}", service_error_text(code)));
        }

        // 轮询等待服务真正停下（最多 8 秒）
        let mut stopped = false;
        for _ in 0..40 {
            if !query_running(svc).unwrap_or(false) {
                stopped = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);

        if stopped {
            Ok(true)
        } else {
            Err(format!("服务 {name} 停止超时，可能被系统策略阻止"))
        }
    }
}

/// 启动服务（用于恢复）。若服务已运行或正在启动，视为成功。
pub fn start_service(name: &str) -> Result<(), String> {
    unsafe {
        let scm = open_scm()?;
        let wname = wide(name);
        let svc = OpenServiceW(
            scm,
            PCWSTR(wname.as_ptr()),
            SERVICE_START | SERVICE_QUERY_STATUS,
        );
        let Ok(svc) = svc else {
            let _ = CloseServiceHandle(scm);
            return Err(format!("无法打开服务 {name}"));
        };

        // 已经在运行就不用启动
        if query_running(svc).unwrap_or(false) {
            let _ = CloseServiceHandle(svc);
            let _ = CloseServiceHandle(scm);
            return Ok(());
        }

        let res = StartServiceW(svc, None);
        if let Err(e) = res {
            let code = e.code().0;
            // 1056 ERROR_SERVICE_ALREADY_RUNNING / 1053 等视为已在运行
            if !is_already_running(code) {
                let _ = CloseServiceHandle(svc);
                let _ = CloseServiceHandle(scm);
                return Err(format!("启动服务 {name} 失败：{}", service_error_text(code)));
            }
        }

        // 等待服务进入运行状态（最多 6 秒）
        for _ in 0..30 {
            if query_running(svc).unwrap_or(false) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);

        // 部分服务（如 WSearch）启动后系统会自动拉起，等待超时但状态可能稍后恢复，
        // 这里不视为失败，交由上层显示实际状态。
        Ok(())
    }
}

fn is_already_running(code: i32) -> bool {
    // 0x80070420 = 1056 ERROR_SERVICE_ALREADY_RUNNING
    // 0x8007041D = 1053 ERROR_SERVICE_REQUEST_TIMEOUT（正在停止中）
    let raw = code as u32;
    raw == 0x80070420 || raw == 0x8007041D
}

/// 把服务相关错误码翻译成中文说明。
fn service_error_text(code: i32) -> String {
    let raw = code as u32;
    let msg = match raw {
        0x80070420 => "服务已在运行",
        0x8007041D => "服务请求超时，可能正在停止",
        0x80070424 => "服务不存在",
        0x80070425 => "服务当前无法接受控制指令",
        0x80070426 => "服务尚未启动",
        0x80070422 => "服务已被禁用，需先在系统服务中改为手动或自动",
        0x8007042C => "服务依赖的其他服务未启动",
        0x8007045B => "系统正在关机",
        0x80070005 => "拒绝访问，需要管理员权限",
        _ => "未知错误",
    };
    format!("{msg}（0x{raw:08X}）")
}

// ---------------------------------------------------------------- 剪贴板 / DNS

/// 清空剪贴板。
pub fn clear_clipboard() -> Result<(), String> {
    unsafe {
        for _ in 0..5 {
            if OpenClipboard(None).is_ok() {
                let res = EmptyClipboard();
                let _ = CloseClipboard();
                return res.map_err(|e| format!("清空剪贴板失败（错误码 {}）", e.code().0));
            }
            // 剪贴板被其他程序占用时短暂重试
            std::thread::sleep(std::time::Duration::from_millis(80));
        }
        Err("剪贴板正被其他程序占用，请稍后重试".into())
    }
}

/// 刷新 DNS 解析缓存。
pub fn flush_dns() -> Result<(), String> {
    unsafe {
        if DnsFlushResolverCache() != 0 {
            Ok(())
        } else {
            Err("刷新 DNS 缓存失败".into())
        }
    }
}

// ---------------------------------------------------------------- 字节格式化

/// 把字节数格式化为 "1.2 GB" 形式。
pub fn format_bytes(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{:.0} KB", b / 1024.0)
    }
}
