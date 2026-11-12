//! 优化项定义与单项执行结果。

use serde::{Deserialize, Serialize};

/// 单个优化项的静态定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeItem {
    pub id: String,
    pub label: String,
    /// 是否需要管理员权限
    pub needs_admin: bool,
    /// 执行后是否可自动恢复
    pub reversible: bool,
}

/// 返回全部优化项的定义。
pub fn item_definitions() -> Vec<OptimizeItem> {
    vec![
        OptimizeItem {
            id: "priority".into(),
            label: "提升游戏进程优先级".into(),
            needs_admin: false,
            reversible: true,
        },
        OptimizeItem {
            id: "background".into(),
            label: "关闭会影响游戏流畅性的后台进程".into(),
            needs_admin: true,
            reversible: false,
        },
        OptimizeItem {
            id: "memory".into(),
            label: "释放电脑内存".into(),
            needs_admin: false,
            reversible: false,
        },
        OptimizeItem {
            id: "wallpaper".into(),
            label: "关闭 Wallpaper Engine".into(),
            needs_admin: false,
            reversible: true,
        },
        OptimizeItem {
            id: "xbox".into(),
            label: "停止 Xbox Live 身份验证管理器服务".into(),
            needs_admin: true,
            reversible: true,
        },
        OptimizeItem {
            id: "search".into(),
            label: "停止 Windows Search 服务".into(),
            needs_admin: true,
            reversible: true,
        },
        OptimizeItem {
            id: "dns".into(),
            label: "清理 DNS".into(),
            needs_admin: false,
            reversible: false,
        },
        OptimizeItem {
            id: "clipboard".into(),
            label: "清理粘贴板内容".into(),
            needs_admin: false,
            reversible: false,
        },
    ]
}

/// 单项执行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// 成功执行
    Done,
    /// 没有可执行目标，正常跳过
    Skip,
    /// 执行失败
    Fail,
}

/// 单项执行结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemResult {
    pub id: String,
    pub status: Status,
    pub detail: String,
    pub elapsed_ms: u64,
}

impl ItemResult {
    /// 是否计入"已完成"（done 与 skip 都算完成，fail 不算）。
    pub fn is_completed(&self) -> bool {
        matches!(self.status, Status::Done | Status::Skip)
    }
}
