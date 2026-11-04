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
    /// 被关闭的程序可执行文件路径（如 Wallpaper Engine），还原时重新启动
    #[serde(default)]
    pub killed_programs: Vec<String>,
}

impl Snapshot {
    pub fn is_empty(&self) -> bool {
        self.demoted.is_empty()
            && self.promoted.is_empty()
            && self.stopped_services.is_empty()
            && self.killed_programs.is_empty()
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
        if !self.killed_programs.is_empty() {
            parts.push(format!("关闭 {} 个程序", self.killed_programs.len()));
        }
        if parts.is_empty() {
            "无可还原的改动".into()
        } else {
            parts.join("，")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_snapshot_reports_nothing_to_restore() {
        let snap = Snapshot::default();
        assert!(snap.is_empty());
        assert_eq!(snap.summary(), "无可还原的改动");
    }

    #[test]
    fn killed_programs_make_snapshot_non_empty() {
        // 关闭程序也是可还原的改动，不能因为只杀了进程就当成"无事发生"
        let snap = Snapshot {
            killed_programs: vec![r"C:\Program Files\Steam\wallpaper64.exe".to_string()],
            ..Default::default()
        };
        assert!(!snap.is_empty());
        assert!(snap.summary().contains("关闭 1 个程序"));
    }

    #[test]
    fn old_config_without_killed_programs_still_deserializes() {
        // 旧版本写的快照没有这个字段，缺省应为空而不是解析失败
        let snap: Snapshot = serde_json::from_str(
            r#"{"demoted":[],"promoted":[],"stopped_services":[]}"#,
        )
        .unwrap();
        assert!(snap.killed_programs.is_empty());
    }
}
