//! 不可见消息窗口：托盘事件的接收方与定时心跳。
//!
//! 没有界面后，托盘仍要求有一个 HWND 接收 `Shell_NotifyIconW` 的回调。
//! 这里创建一个带 `WS_EX_TOOLWINDOW`、从不 Show 的普通顶层窗口——它不
//! 参与绘制，也不会出现在任务栏或 Alt+Tab 里。
//!
//! 不能用 `HWND_MESSAGE`（message-only）窗口：它收不到系统广播，
//! Explorer 重启后的 `TaskbarCreated` 到不了，托盘图标会一直缺失。
//!
//! 主循环结构：`SetTimer` 每 200 ms 触发一次 `WM_TIMER`，在窗口过程里
//! 消费共享状态并刷新托盘。所有 UI 操作都发生在同一个线程上，因此不
//! 需要跨线程的消息投递。

use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetTimer, TranslateMessage, HMENU, MSG,
    WM_APP, WM_CLOSE, WM_DESTROY, WM_HOTKEY, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

use crate::logging;
use crate::tray::WM_TRAY;
use crate::winapi::wide;

/// 跨线程请求自定义消息（`WM_APP + 2`）。
pub const WM_WGOPT_REQUEST: u32 = WM_APP + 2;
/// 心跳定时器 ID。
const TIMER_ID: usize = 1;
/// 心跳间隔：足够快，又不空转 CPU。
const TIMER_MS: u32 = 200;

/// Explorer 重启后广播的消息。
///
/// 这个 ID 由 `RegisterWindowMessageW("TaskbarCreated")` 动态分配，不能写死。
/// 之前写死成 `0x001E` 恰好是 `WM_TIMECHANGE`，于是「Explorer 重启后重新
/// 注册托盘」永远不会触发，只有改系统时间时才误触发一次。
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn taskbar_created_message() -> u32 {
    TASKBAR_CREATED.load(Ordering::SeqCst)
}

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

/// 共享的消息窗口句柄。
pub fn main_hwnd() -> Option<HWND> {
    let v = MAIN_HWND.load(Ordering::SeqCst);
    (v != 0).then_some(HWND(v as *mut _))
}

/// 窗口过程回调集合。
pub struct Callbacks {
    /// 心跳：每 200 ms 一次，用于刷新托盘与弹通知
    pub tick: Box<dyn FnMut()>,
    /// 请求弹菜单
    pub show_menu: Box<dyn FnMut()>,
    /// Explorer 重启后重新注册托盘
    pub reattach: Box<dyn FnMut()>,
    /// 全局快捷键按下（「立即开始优化」）
    pub hotkey: Box<dyn FnMut()>,
}

/// 回调全局槽位。
///
/// 窗口过程必须是 `extern "system"` 自由函数，无法捕获闭包；因此把回调
/// 存在静态槽里，只在主线程访问，不存在竞争。
static mut CALLBACKS: Option<Callbacks> = None;

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER if wparam.0 == TIMER_ID => {
                if let Some(cb) = &mut *std::ptr::addr_of_mut!(CALLBACKS) {
                    (cb.tick)();
                }
                LRESULT(0)
            }
            WM_TRAY => {
                // 鼠标消息在 lParam 的低字里；左键、右键都弹菜单，
                // 因为没有界面窗口可以打开
                let mouse = (lparam.0 as u32) & 0xFFFF;
                if mouse == 0x0202 || mouse == 0x0205 {
                    if let Some(cb) = &mut *std::ptr::addr_of_mut!(CALLBACKS) {
                        (cb.show_menu)();
                    }
                }
                LRESULT(0)
            }
            WM_HOTKEY => {
                // 只认自己注册的那个 id，别把别人的热键当成本程序的
                if wparam.0 as i32 == crate::hotkey::HOTKEY_ID {
                    if let Some(cb) = &mut *std::ptr::addr_of_mut!(CALLBACKS) {
                        (cb.hotkey)();
                    }
                }
                LRESULT(0)
            }
            WM_WGOPT_REQUEST => {
                if let Some(cb) = &mut *std::ptr::addr_of_mut!(CALLBACKS) {
                    match wparam.0 {
                        0 => (cb.show_menu)(),
                        1 => (cb.reattach)(),
                        _ => {}
                    }
                }
                LRESULT(0)
            }
            // 广播消息：Explorer 重启（或首次就绪）后重新注册托盘图标
            m if m != 0 && m == taskbar_created_message() => {
                if let Some(cb) = &mut *std::ptr::addr_of_mut!(CALLBACKS) {
                    (cb.reattach)();
                }
                LRESULT(0)
            }
            WM_CLOSE | WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// 创建消息窗口但不启动循环。
///
/// 返回句柄后调用方可以先注册托盘图标，再调用 [`run_message_loop`]。
///
/// 窗口必须是**普通顶层窗口**（只是从不 Show），不能用 `HWND_MESSAGE`：
/// message-only 窗口收不到系统广播，Explorer 重启后发的 `TaskbarCreated`
/// 就到不了这里，托盘图标会一直缺失到下次启动。
pub fn create_message_window() -> Result<HWND, String> {
    let class_name = wide("WGOptCoreMessageWindow");

    // Explorer 重启广播的消息 ID：必须注册后才拿到真实值。
    // 注册失败时保持 0，窗口过程里 `m != 0` 的判断会跳过。
    let taskbar_msg = unsafe { RegisterWindowMessageW(PCWSTR(wide("TaskbarCreated").as_ptr())) };
    TASKBAR_CREATED.store(taskbar_msg, Ordering::SeqCst);

    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| format!("获取模块句柄失败：{e}"))?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            // 已注册过（重复初始化）不是致命错误
            logging::warn("RegisterClassW 返回 0（可能已注册）");
        }

        let title = wide("WGOpt");
        // 不创建为 message-only 窗口（理由见函数注释）。加上 WS_EX_TOOLWINDOW
        // 保证它永远不进任务栏与 Alt+Tab，即使将来被误 Show 也一样。
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None::<HMENU>,
            Some(instance.into()),
            None,
        )
        .map_err(|e| format!("创建消息窗口失败：{e}"))?;

        MAIN_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        Ok(hwnd)
    }
}

/// 安装回调并运行消息循环，直到窗口销毁。
pub fn run_message_loop(callbacks: Callbacks) {
    unsafe {
        CALLBACKS = Some(callbacks);

        if let Some(hwnd) = main_hwnd() {
            if SetTimer(Some(hwnd), TIMER_ID, TIMER_MS, None) == 0 {
                logging::warn("SetTimer 失败，托盘刷新可能延迟");
            }
        }

        let mut msg = MSG::default();
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 == 0 || ret.0 == -1 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        CALLBACKS = None;
        MAIN_HWND.store(0, Ordering::SeqCst);
    }
}

/// 请求主线程弹菜单（供外部实例唤起已有进程时使用）。
pub fn post_show_menu() {
    let hwnd = main_hwnd().or_else(|| unsafe {
        // 另一个进程刚启动时 main_hwnd() 还没赋值：按窗口类名找一次。
        // 第二个实例走的就是这条路径。
        let class = wide("WGOptCoreMessageWindow");
        FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()).ok()
    });
    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_WGOPT_REQUEST, WPARAM(0), LPARAM(0));
        }
    }
}

/// 请求主线程退出。
pub fn post_quit() {
    if let Some(hwnd) = main_hwnd() {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

