//! 单实例互斥：第二个实例启动时唤起已有实例的菜单，而不是重复常驻。
//!
//! 用命名互斥体 + 命名事件实现：
//! - 互斥体判断是否已有实例；
//! - 已存在时通过命名事件通知对方弹出托盘菜单，然后退出。

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject};

const MUTEX_NAME: &str = r"Local\WGOpt.Core.Mutex";
const SHOW_EVENT_NAME: &str = r"Local\WGOpt.Core.ShowMenu";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 单实例守卫：持有互斥体句柄直到进程退出。
pub struct SingleInstance {
    _mutex: HANDLE,
}

/// 尝试成为唯一实例。
///
/// 返回 `Some(guard)` 表示本进程是首实例；
/// 返回 `None` 表示已有实例在运行（已通知它弹菜单）。
pub fn acquire() -> Option<SingleInstance> {
    let name = wide(MUTEX_NAME);
    unsafe {
        let mutex = CreateMutexW(None, true, PCWSTR(name.as_ptr())).ok()?;
        let err = GetLastError();
        if err == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            signal_existing();
            return None;
        }
        Some(SingleInstance { _mutex: mutex })
    }
}

/// 通知已运行的实例弹出菜单。
fn signal_existing() {
    let name = wide(SHOW_EVENT_NAME);
    unsafe {
        if let Ok(event) = CreateEventW(None, false, false, PCWSTR(name.as_ptr())) {
            let _ = SetEvent(event);
            let _ = CloseHandle(event);
        }
    }
}

/// 打开（或创建）用于接收"请弹菜单"信号的事件。
pub fn open_show_event() -> Option<HANDLE> {
    let name = wide(SHOW_EVENT_NAME);
    unsafe {
        CreateEventW(None, false, false, PCWSTR(name.as_ptr())).ok()
    }
}

/// 非阻塞地检查事件是否被触发。
pub fn show_event_signaled(event: HANDLE) -> bool {
    unsafe { WaitForSingleObject(event, 0) == WAIT_OBJECT_0 }
}
