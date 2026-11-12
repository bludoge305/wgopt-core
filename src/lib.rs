//! WGOpt 核心：无界面的游戏优化后台程序。
//!
//! 与原版 WGOpt 桌面版共用同一套优化机制、白名单与快照还原：
//! 8 项优化、进程/服务白名单、快照与还原。
//! 只是把界面换成系统托盘常驻，功能开关改由配置文件与托盘菜单控制；
//! 优化不再由游戏检测自动触发，只在用户明确要求时执行。
//!
//! 模块职责：
//! - [`winapi`]：Win32 能力封装（进程、服务、内存、剪贴板、DNS）
//! - [`whitelist`]：允许操作的白名单与硬性排除
//! - [`item`]：优化项定义与结果类型
//! - [`snapshot`]：可还原改动的快照
//! - [`engine`]：8 项优化与还原
//! - [`focus`]：前台上游进程捕获与排除清单判定
//! - [`hotkey`]：全局快捷键（`RegisterHotKey`）与编辑场景判定
//! - [`config`]：配置文件读写与开机自启
//! - [`app`]：后台工作线程与手动优化/还原
//! - [`tray`]：托盘图标、菜单与通知
//! - [`window`]：不可见消息窗口与消息循环
//! - [`single_instance`]：单实例互斥
//! - [`logging`]：文件日志

pub mod app;
pub mod config;
pub mod elevate;
pub mod engine;
pub mod focus;
pub mod hotkey;
pub mod item;
pub mod logging;
pub mod single_instance;
pub mod snapshot;
pub mod tray;
pub mod whitelist;
pub mod winapi;
pub mod window;

pub use item::{ItemResult, OptimizeItem, Status as ItemStatus};
pub use snapshot::Snapshot;

/// 程序版本，与 Cargo.toml 保持一致。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 读取全部优化项定义。
pub fn item_definitions() -> Vec<OptimizeItem> {
    item::item_definitions()
}
