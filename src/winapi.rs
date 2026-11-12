//! Windows 系统能力封装：进程、服务、内存、剪贴板、DNS。
//!
//! 原样沿用 WGOpt 桌面版的实现，只去掉界面需要的展示字段。
//! 所有对外函数返回 `Result<_, String>`，错误信息为可直接展示的中文。

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx,
    StartServiceW, SC_HANDLE, SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_QUERY_STATUS,
    SERVICE_RUNNING, SERVICE_START, SERVICE_STATUS, SERVICE_STATUS_PROCESS, SERVICE_STOP,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetPriorityClass, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_INFORMATION,
    SetPriorityClass, TerminateProcess, ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
    HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SET_QUOTA,
    PROCESS_TERMINATE, REALTIME_PRIORITY_CLASS,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};

// 刷新 DNS 缓存：`windows` crate 0.62 没有导出 `DnsFlushResolverCache`
// （win32metadata 里缺这一条），只能自己去 dnsapi.dll 取。
//
// 这里走运行时 `GetProcAddress` 而不是 `#[link(name = "dnsapi")]`：后者要求
// 链接器能找到 `libdnsapi.a`，而 MinGW 发行版未必都带（本机就没有），
// 于是「装好 Rust 就能编」变成「还得凑齐导入库」。`dnsapi.dll` 是系统
// 自带的，取地址不会失败。
fn dns_flush_resolver_cache() -> Option<unsafe extern "system" fn() -> u32> {
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};

    unsafe {
        // dnsapi.dll 未必已经加载进本进程（只有解析过 DNS 的进程才有），
        // 所以先用 GetModuleHandle 探一下，拿不到就 LoadLibrary。
        // 句柄故意不释放：进程退出时系统会回收，这里也省得处理
        // 「释放后函数指针又活了一会儿」的问题。
        let lib = GetModuleHandleW(windows::core::w!("dnsapi.dll"))
            .or_else(|_| LoadLibraryW(windows::core::w!("dnsapi.dll")))
            .ok()?;
        let proc = GetProcAddress(lib, windows::core::s!("DnsFlushResolverCache"))?;
        // FARPROC 是泛型函数指针，转成具体的 DnsFlushResolverCache 签名
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn() -> u32,
        >(proc))
    }
}

/// 将 Rust 字符串转成以 NUL 结尾的宽字符串。
pub fn wide(s: &str) -> Vec<u16> {
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

/// 轻量版进程列表：只取 pid 与名称，不做句柄/内存查询。
/// 批量结束进程、调整优先级只需要这两项。
pub fn list_processes_basic() -> Vec<(u32, String)> {
    process_snapshot_raw()
        .into_iter()
        .map(|(pid, _, name)| (pid, name))
        .collect()
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

/// 设置进程优先级，返回设置前的优先级名。
///
/// 句柄同时申请 `PROCESS_SET_INFORMATION` 与 `PROCESS_QUERY_INFORMATION`：
/// 只有前者时 `GetPriorityClass` 会失败并返回 0，原来记录的「原优先级」
/// 就永远是「未知」，还原时又因「未知」不是合法等级而静默失败。
pub fn set_process_priority(pid: u32, level: &str) -> Result<String, String> {
    let Some(class) = priority_class(level) else {
        return Err(format!("未知的优先级：{level}"));
    };
    unsafe {
        let access = PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION;
        let handle = OpenProcess(access, false, pid).map_err(|e| {
            format!("无法打开进程 {pid}（错误码 {}），可能需要管理员权限", e.code().0)
        })?;
        let before_raw = GetPriorityClass(handle);
        let before = priority_name(before_raw);
        if before_raw == 0 {
            let code = GetLastError();
            let _ = CloseHandle(handle);
            return Err(format!(
                "读取进程 {pid} 当前优先级失败（错误码 0x{:08X}）",
                code.0
            ));
        }
        let res = SetPriorityClass(handle, class);
        let _ = CloseHandle(handle);
        res.map_err(|e| format!("设置优先级失败（错误码 {}）", e.code().0))?;
        Ok(before.to_string())
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

/// 前台窗口所属进程：(pid, 进程名)。拿不到时返回 `None`。
///
/// 用于快捷键触发时确认「用户此刻在玩哪个程序」。`GetForegroundWindow`
/// 返回无效句柄（桌面、锁屏、切换中）或窗口不属于任何进程时返回 `None`。
pub fn foreground_process() -> Option<(u32, String)> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let name = process_name(pid);
        if name.is_empty() {
            return None;
        }
        Some((pid, name))
    }
}

/// 取进程名（不含路径）。取不到时返回空串。
pub fn process_name(pid: u32) -> String {
    let path = process_image_path(pid);
    if path.is_empty() {
        return String::new();
    }
    std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
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

    // 只需要 pid：工作集会在 EmptyWorkingSet 之后由系统重算，
    // 事先查一遍是白花一次全量进程统计。
    for (pid, _) in list_processes_basic() {
        unsafe {
            let Ok(handle) =
                OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
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
        if let Err(e) = ControlService(svc, SERVICE_CONTROL_STOP_CODE, &mut status) {
            let code = e.code().0;
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
    let Some(flush) = dns_flush_resolver_cache() else {
        return Err("找不到 dnsapi.dll 的刷新接口".into());
    };
    // 返回非零表示成功（与原版一致）
    if unsafe { flush() } != 0 {
        Ok(())
    } else {
        Err("刷新 DNS 缓存失败".into())
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
