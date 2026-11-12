//! 日志：写入 `%APPDATA%\WGOpt\wgopt.log`，按大小滚动。
//!
//! 常驻后台程序没有界面，出问题时这份日志是唯一的排查依据。
//! 单文件上限 512 KB，超限时把旧日志改名为 `.1`（只保留一份历史）。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_BYTES: u64 = 512 * 1024;

static LOGGER: Mutex<Option<PathBuf>> = Mutex::new(None);

/// 初始化日志文件路径。
pub fn init(dir: &std::path::Path) {
    let _ = fs::create_dir_all(dir);
    let path = dir.join("wgopt.log");
    rotate_if_needed(&path);
    if let Ok(mut guard) = LOGGER.lock() {
        *guard = Some(path);
    }
}

fn rotate_if_needed(path: &std::path::Path) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.len() < MAX_BYTES {
        return;
    }
    let old = path.with_extension("log.1");
    let _ = fs::remove_file(&old);
    let _ = fs::rename(path, &old);
}

/// 写一行日志。失败时静默——日志本身不该拖垮主流程。
pub fn write(level: &str, message: &str) {
    let stamp = timestamp();
    let line = format!("[{stamp}] {level:<5} {message}\n");

    // 非 release 构建同时输出到控制台，便于开发时观察
    if cfg!(debug_assertions) {
        print!("{line}");
        let _ = std::io::stdout().flush();
    }

    let path = LOGGER
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned());
    let Some(path) = path else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

pub fn info(message: &str) {
    write("INFO", message);
}

pub fn warn(message: &str) {
    write("WARN", message);
}

pub fn error(message: &str) {
    write("ERROR", message);
}

/// 本地时间字符串（秒级）。
fn timestamp() -> String {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::GetLocalTime;
        let t = unsafe { GetLocalTime() };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
        )
    }
    #[cfg(not(windows))]
    {
        String::from("0000-00-00 00:00:00")
    }
}

/// 当前日志文件路径，供托盘菜单打开目录时使用。
pub fn log_path() -> PathBuf {
    LOGGER
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| crate::config::config_dir().join("wgopt.log"))
}
