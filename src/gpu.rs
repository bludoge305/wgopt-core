//! GPU 占用采集：通过 Windows 性能计数器读取每个进程的 GPU 引擎占用。
//!
//! 这是任务管理器、RTSS、MSI Afterburner 显示「每进程 GPU 使用率」的同一数据源
//! （计数器路径 `\GPU Engine(*)\Utilization Percentage`）。
//!
//! 相比进程名匹配或窗口尺寸猜测，GPU 占用是最强、最可靠的游戏信号：
//! 游戏的 3D 引擎通常独占 30%~100%，而浏览器、编辑器、聊天软件即使
//! 全屏播放视频也只有个位数。实测同机对比：游戏 62.5%，第二名 1.1%。

use std::collections::HashMap;

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
        PdhGetFormattedCounterArrayW, PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
        PDH_HCOUNTER, PDH_HQUERY,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// GPU 采集器：持有查询句柄，跨轮询复用。
    ///
    /// PDH 的 Utilization Percentage 需要两次采样才能算出速率，
    /// 因此第一次 `collect()` 只建立基线，之后才有有效数据。
    pub struct GpuSampler {
        query: PDH_HQUERY,
        counter: PDH_HCOUNTER,
        primed: bool,
    }

    impl GpuSampler {
        pub fn new() -> Option<Self> {
            unsafe {
                let mut query = PDH_HQUERY::default();
                if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                    return None;
                }

                let path = wide(r"\GPU Engine(*)\Utilization Percentage");
                let mut counter = PDH_HCOUNTER::default();
                if PdhAddEnglishCounterW(query, PCWSTR(path.as_ptr()), 0, &mut counter) != 0 {
                    let _ = PdhCloseQuery(query);
                    return None;
                }

                // 建立基线
                let _ = PdhCollectQueryData(query);

                Some(Self {
                    query,
                    counter,
                    primed: false,
                })
            }
        }

        /// 采集一轮，返回 pid -> 该进程所有引擎占用之和（百分比）。
        pub fn collect(&mut self) -> HashMap<u32, f32> {
            let mut out = HashMap::new();
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return out;
                }
                // 首轮数据是"自进程启动以来"的累计值，无参考意义
                if !self.primed {
                    self.primed = true;
                    return out;
                }

                // 先取所需缓冲区大小
                let mut size = 0u32;
                let mut count = 0u32;
                let _ = PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut size,
                    &mut count,
                    None,
                );
                if size == 0 || count == 0 {
                    return out;
                }

                let item_size = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
                let n = (size as usize) / item_size + 1;
                let mut buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> = Vec::with_capacity(n);
                buf.resize_with(n, || std::mem::zeroed());

                let mut size2 = size;
                let mut count2 = count;
                let rc = PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut size2,
                    &mut count2,
                    Some(buf.as_mut_ptr()),
                );
                if rc != 0 {
                    return out;
                }

                for item in buf.iter().take(count2 as usize) {
                    if item.szName.is_null() {
                        continue;
                    }
                    let name = pcwstr_to_string(item.szName.0);
                    // 实例名形如 pid_31340_luid_0x00000000_0x0000C4A9_phys_0_eng_0_engtype_3D
                    let Some(pid) = parse_pid(&name) else {
                        continue;
                    };
                    let value = item.FmtValue.Anonymous.doubleValue;
                    if value.is_finite() && value > 0.0 {
                        *out.entry(pid).or_insert(0.0) += value as f32;
                    }
                }
            }
            out
        }
    }

    // PDH 句柄是裸指针，本身不满足 Send。但采样器始终被 Mutex 包裹、
    // 串行访问，不存在并发调用，因此可以安全地跨线程移动。
    unsafe impl Send for GpuSampler {}

    impl Drop for GpuSampler {
        fn drop(&mut self) {
            unsafe {
                let _ = PdhCloseQuery(self.query);
            }
        }
    }

    unsafe fn pcwstr_to_string(p: *const u16) -> String {
        let mut len = 0;
        while *p.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }

    /// 从 PDH 实例名中解析出 pid。
    fn parse_pid(instance: &str) -> Option<u32> {
        let start = instance.find("pid_")? + 4;
        let rest = &instance[start..];
        let end = rest.find('_').unwrap_or(rest.len());
        rest[..end].parse().ok()
    }
}

#[cfg(windows)]
pub use imp::GpuSampler;

#[cfg(not(windows))]
pub struct GpuSampler;

#[cfg(not(windows))]
impl GpuSampler {
    pub fn new() -> Option<Self> {
        None
    }
    pub fn collect(&mut self) -> HashMap<u32, f32> {
        HashMap::new()
    }
}
