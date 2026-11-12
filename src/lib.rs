//! WGOpt 核心：无界面的游戏优化后台程序。
//!
//! 与原版 WGOpt 桌面版共用同一套优化机制、白名单与游戏识别：
//! 8 项优化、进程/服务白名单、快照与还原、GPU 性能计数器识别游戏。
//! 只是把界面换成系统托盘常驻，功能开关改由配置文件与托盘菜单控制。
//!
//! 模块职责：
//! - [`winapi`]：Win32 能力封装（进程、服务、内存、剪贴板、DNS）
//! - [`whitelist`]：允许操作的白名单与硬性排除
//! - [`item`]：优化项定义与结果类型
//! - [`snapshot`]：可还原改动的快照
//! - [`engine`]：8 项优化与还原
//! - [`gpu`]：GPU 占用采集（PDH 性能计数器）
//! - [`game_detect`]：游戏识别（GPU 为主判据 + 多维加权）
//! - [`hotkey`]：全局快捷键（`RegisterHotKey`）与编辑场景判定
//! - [`config`]：配置文件读写与开机自启
//! - [`app`]：后台守护状态机与自动优化
//! - [`tray`]：托盘图标、菜单与通知
//! - [`window`]：不可见消息窗口与消息循环
//! - [`single_instance`]：单实例互斥
//! - [`logging`]：文件日志

pub mod app;
pub mod config;
pub mod elevate;
pub mod engine;
pub mod game_detect;
pub mod gpu;
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
