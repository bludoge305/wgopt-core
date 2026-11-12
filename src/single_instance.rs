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

use crate::winapi::wide;

const MUTEX_NAME: &str = r"Local\WGOpt.Core.Mutex";
const SHOW_EVENT_NAME: &str = r"Local\WGOpt.Core.ShowMenu";

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

/// 持有「请弹菜单」事件，供消息循环线程等待。
///
/// 首实例必须在启动时创建这个事件：第二个实例靠 `SetEvent` 通知它弹菜单。
/// 之前只在 `signal_existing` 里临时 `CreateEventW` 再立刻关掉，首实例
/// 从来没等过它，所以「再次启动程序会弹出已有实例的菜单」一直没生效。
pub struct ShowEvent(HANDLE);

impl ShowEvent {
    pub fn open() -> Option<Self> {
        let name = wide(SHOW_EVENT_NAME);
        // 自动复位事件：一次 SetEvent 只唤醒一次等待
        unsafe { CreateEventW(None, false, false, PCWSTR(name.as_ptr())).ok().map(Self) }
    }

    /// 等最多 `timeout_ms` 毫秒；被通知返回 true。
    pub fn wait(&self, timeout_ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.0, timeout_ms) == WAIT_OBJECT_0 }
    }
}

impl Drop for ShowEvent {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
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
