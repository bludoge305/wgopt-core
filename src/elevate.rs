//! 管理员权限：与原版一致，运行时用 `runas` 重新拉起自身。
//!
//! 之所以不用 application manifest 里的 `requireAdministrator`：MinGW 链接
//! 时 Rust 已经嵌入了一份默认 manifest，再嵌一份会让 ld 报
//! `.rsrc merge failure: multiple non-default manifests`。原版 WGOpt 采用的
//! 正是运行时提权，这里保持同样的做法。

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED};
use windows::Win32::UI::Shell::{
    IsUserAnAdmin, ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};

/// 当前进程是否已在管理员权限下运行。
pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin().as_bool() }
}

/// 以管理员身份重新启动自身。
///
/// 返回 `Ok(true)` 表示新实例已拉起，调用方应立即退出；
/// `Ok(false)` 表示用户取消了 UAC。
pub fn relaunch_elevated() -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| format!("无法获取程序路径：{e}"))?;

    // 原样透传外部参数（如 --autostart）
    let args: Vec<String> = std::env::args().skip(1).collect();

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };

    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(&args.join(" "));
    let dir = wide(
        &exe.parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    );

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        // 不用 NO_UI，否则 UAC 被拒绝时拿不到明确错误
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: PCWSTR(dir.as_ptr()),
        nShow: 1, // SW_SHOWNORMAL
        ..Default::default()
    };

    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(_) => {
            if !info.hProcess.is_invalid() {
                unsafe {
                    let _ = CloseHandle(info.hProcess);
                }
            }
            Ok(true)
        }
        Err(_) => {
            let code = unsafe { GetLastError() };
            if code == ERROR_CANCELLED {
                Ok(false)
            } else {
                Err(format!("启动管理员实例失败（错误码 0x{:08X}）", code.0))
            }
        }
    }
}

/// 弹出消息框询问是否重试提权。返回 true 表示用户选择「重试」。
pub fn ask_retry(text: &str, caption: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDRETRY, MB_ICONWARNING, MB_RETRYCANCEL, MB_SETFOREGROUND, MB_TOPMOST,
    };

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let t = wide(text);
    let c = wide(caption);
    let style = MB_RETRYCANCEL | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND;
    unsafe {
        let r = MessageBoxW(None, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), style);
        r == IDRETRY
    }
}

/// 保证当前进程拥有管理员权限；必要时提权重启。
///
/// 提权失败且用户放弃时直接结束进程。
pub fn ensure_admin() {
    if is_admin() {
        return;
    }

    loop {
        match relaunch_elevated() {
            Ok(true) => {
                // 管理员实例已启动，当前进程静默退出
                std::process::exit(0);
            }
            Ok(false) => {
                let retry = ask_retry(
                    "WGOpt 需要管理员权限才能正常工作。\n\n\
                     以下功能在普通权限下会失败：\n\
                     · 停止系统服务（Xbox、Windows Search）\n\
                     · 结束其他用户的后台进程\n\
                     · 调整游戏进程优先级\n\n\
                     是否重新请求管理员权限？\n\
                     选择「重试」再次授权，选择「取消」退出程序。",
                    "WGOpt · 需要管理员权限",
                );
                if retry {
                    continue;
                }
                std::process::exit(1);
            }
            Err(msg) => {
                ask_retry(
                    &format!("{msg}\n\n请尝试右键点击程序图标，选择「以管理员身份运行」。"),
                    "WGOpt · 启动失败",
                );
                std::process::exit(1);
            }
        }
    }
}
