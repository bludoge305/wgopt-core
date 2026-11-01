//! 快照：记录可还原的改动，停止优化时按此恢复。

use serde::{Deserialize, Serialize};

/// 优先级快照：记录为了优化而降级/提升的进程与停止的服务，便于还原。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snapshot {
    /// 被降级的后台进程 (pid, 进程名, 原优先级)
    pub demoted: Vec<(u32, String, String)>,
    /// 被提升的游戏进程 (pid, 进程名)
    pub promoted: Vec<(u32, String)>,
    /// 被停止的服务名
    pub stopped_services: Vec<String>,
}

impl Snapshot {
    pub fn is_empty(&self) -> bool {
        self.demoted.is_empty() && self.promoted.is_empty() && self.stopped_services.is_empty()
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.promoted.is_empty() {
            parts.push(format!("提升 {} 个游戏进程", self.promoted.len()));
        }
        if !self.demoted.is_empty() {
            parts.push(format!("降级 {} 个后台进程", self.demoted.len()));
        }
        if !self.stopped_services.is_empty() {
            parts.push(format!("停止 {} 个服务", self.stopped_services.len()));
        }
        if parts.is_empty() {
            "无可还原的改动".into()
        } else {
            parts.join("，")
        }
    }
}
