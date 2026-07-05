//! Agent 核心数据结构
//! - [`AgentClass`] — 员工类型（OneShot / Interactive / Scheduled）
//! - [`AgentStatus`] — 运行状态
//! - [`AgentSpec`] — 创建规格（Boss 传给 CreateAgent 的参数）
//! - [`AgentReport`] — 员工向 Boss 的汇报
//! - [`AgentEntry`] — 员工条目（封装 StateMachine）
//! - [`AgentPool`] — 员工池

use corework::statemachine::StateMachine;
use corework::world::FrameworkState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// 员工类型
// ============================================================================

/// 员工执行类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentClass {
    /// 一次性执行，完成后自毁
    OneShot,

    Interactive,

    /// 定时心跳，执行绑定的工作流
    /// interval_secs: 心跳间隔（秒）
    Scheduled {
        interval_secs: u64,
    },
}

impl AgentClass {
    pub fn is_interactive(&self) -> bool {
        matches!(self, AgentClass::Interactive)
    }

    pub fn is_scheduled(&self) -> bool {
        matches!(self, AgentClass::Scheduled { .. })
    }

    /// 是否占用对话焦点（Interactive 独占焦点，其余后台运行）
    pub fn occupies_focus(&self) -> bool {
        matches!(self, AgentClass::Interactive)
    }

    /// 从字符串解析 AgentClass（不含 interval，Scheduled 默认 300s）
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "oneshot" | "one_shot" => AgentClass::OneShot,
            "interactive" => AgentClass::Interactive,
            "scheduled" => AgentClass::Scheduled { interval_secs: 300 },
            _ => AgentClass::OneShot,
        }
    }
}

// ============================================================================
// 员工状态
// ============================================================================

/// 员工运行状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentStatus {
    /// 已创建，状态机尚未启动（创建后到 start 前的短暂状态；恢复时也用此状态）
    Pending,
    /// 运行中
    Running,
    /// 暂停（等待用户输入 / Boss 指令）
    Idle,
    /// 已完成
    Completed { summary: String },
    /// 执行失败
    Failed { error: String },
    /// 主动取消（用户或 Boss 取消，历史保留但不再恢复）
    Canceled,
}

impl AgentStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, AgentStatus::Running | AgentStatus::Idle)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            AgentStatus::Completed { .. } | AgentStatus::Failed { .. } | AgentStatus::Canceled
        )
    }
}

// ============================================================================
// 员工汇报
// ============================================================================

/// 员工向 Boss 的汇报类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentReport {
    /// 任务完成
    Completed {
        summary: String,
        /// 产出物（如录制的工作流名、采集的数据等）
        artifacts: Vec<String>,
    },

    /// 需要 Boss 帮助（超出职责范围 / 需要权限）
    NeedHelp {
        reason: String,
        /// 需要的工具或 skill 名称
        requested_tools: Vec<String>,
    },

    /// 任务失败
    Failed {
        error: String,
        /// 建议的重试方式
        retry_suggestion: Option<String>,
    },

    /// 定时心跳汇报（Scheduled 员工正常执行后）
    Heartbeat {
        status: String,
        data: serde_json::Value,
    },
}

impl AgentReport {
    /// 转为 Boss 对话历史中的消息文本
    pub fn to_boss_message(&self, agent_name: &str) -> String {
        match self {
            AgentReport::Completed { summary, artifacts } => {
                let artifacts_str = if artifacts.is_empty() {
                    String::new()
                } else {
                    format!("\n产出物：{}", artifacts.join(", "))
                };
                format!("[员工复命: {}] {}{}", agent_name, summary, artifacts_str)
            }
            AgentReport::NeedHelp {
                reason,
                requested_tools,
            } => {
                let tools_str = if requested_tools.is_empty() {
                    String::new()
                } else {
                    format!("\n需要的工具权限：{}", requested_tools.join(", "))
                };
                format!("[员工请求: {}] {}{}", agent_name, reason, tools_str)
            }
            AgentReport::Failed {
                error,
                retry_suggestion,
            } => {
                let retry_str = retry_suggestion
                    .as_ref()
                    .map(|s| format!("\n建议：{}", s))
                    .unwrap_or_default();
                format!("[员工异常: {}] {}{}", agent_name, error, retry_str)
            }
            AgentReport::Heartbeat { status, data } => {
                format!(
                    "[员工心跳: {}] 状态: {}，数据: {}",
                    agent_name, status, data
                )
            }
        }
    }
}

// ============================================================================
// 创建规格
// ============================================================================

/// 创建员工的规格（Boss 通过 CreateAgent 工具传入）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    /// 显示名（如"录制助手"、"抖音监控"）
    pub name: String,
    /// 员工类型
    pub class: AgentClass,
    /// 注入的 skill 名称列表
    pub skills: Vec<String>,
    /// 额外工具（除 skills 声明的工具外，Boss 额外授权的）
    #[serde(default)]
    pub extra_tools: Vec<String>,
    /// 绑定的工作流名（Scheduled 类常用）
    #[serde(default)]
    pub workflow: Option<String>,
    /// 初始任务描述（OneShot/Interactive 的第一条 user message）
    #[serde(default)]
    pub intent: String,
    /// 自定义 persona 补充（追加到自动生成的 persona 后面）
    #[serde(default)]
    pub persona_extra: Option<String>,
}

// ============================================================================
// 员工条目（封装 StateMachine）
// ============================================================================

/// 员工条目 —— 持有一个 StateMachine 实例
/// 所有业务状态（persona / skills / tools / history）均存在状态机的 ScopedCache 里，
/// 条目本身只保存身份信息和状态机引用。
pub struct AgentEntry {
    pub id: String,
    pub name: String,
    pub class: AgentClass,
    pub sm: Arc<StateMachine>,
}

impl AgentEntry {
    /// 当前状态机状态名
    pub fn current_state(&self) -> String {
        self.sm.current_state()
    }
}

// ============================================================================
// 员工信息（序列化友好，供 ListAgents / 前端展示）
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub class: AgentClass,
    pub current_state: String,
}

// ============================================================================
// 员工池
// ============================================================================

/// 管理所有活跃的子 Agent
pub struct AgentPool {
    /// 员工条目 (id → AgentEntry)
    entries: HashMap<String, AgentEntry>,
    /// 下一个员工 ID 序号
    next_id: u64,
}

impl AgentPool {
    /// 创建空的员工池
    pub fn new(_framework: FrameworkState) -> Self {
        Self {
            entries: HashMap::new(),
            next_id: 1,
        }
    }

    /// 生成下一个 agent_id（同时递增计数器）
    pub fn next_id(&mut self) -> String {
        let id = format!("agent_{}", self.next_id);
        self.next_id += 1;
        id
    }

    /// 注册 Agent（StateMachine 已 start，直接放入池中）
    pub fn register(
        &mut self,
        agent_id: String,
        sm: Arc<StateMachine>,
        class: AgentClass,
        name: String,
    ) {
        let entry = AgentEntry {
            id: agent_id.clone(),
            name,
            class,
            sm,
        };
        self.entries.insert(agent_id, entry);
    }

    /// 获取员工条目（不可变）
    pub fn get_entry(&self, id: &str) -> Option<&AgentEntry> {
        self.entries.get(id)
    }

    /// 获取员工条目（可变）
    pub fn get_entry_mut(&mut self, id: &str) -> Option<&mut AgentEntry> {
        self.entries.get_mut(id)
    }

    /// 按名称查找 agent_id
    pub fn find_id_by_name(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(_, e)| e.name == name)
            .map(|(id, _)| id.as_str())
    }

    /// 按名称销毁员工，返回被销毁的 agent_id
    pub fn dismiss_by_name(&mut self, name: &str) -> Option<String> {
        if let Some(id) = self.find_id_by_name(name).map(|s| s.to_string()) {
            self.entries.remove(&id);
            tracing::debug!(agent_id = %id, agent_name = %name, "agent pool destroyed agent");
            return Some(id);
        }
        None
    }

    /// 销毁员工（按 id）
    pub fn dismiss(&mut self, id: &str) -> bool {
        if self.entries.remove(id).is_some() {
            tracing::debug!(agent_id = %id, "agent pool destroyed agent");
            true
        } else {
            false
        }
    }

    /// 员工数量
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// 列出所有员工信息
    pub fn list(&self) -> Vec<AgentInfo> {
        self.entries
            .values()
            .map(|e| AgentInfo {
                id: e.id.clone(),
                name: e.name.clone(),
                class: e.class.clone(),
                current_state: e.sm.current_state(),
            })
            .collect()
    }

    /// 遍历所有条目（只读）
    pub fn iter(&self) -> impl Iterator<Item = &AgentEntry> {
        self.entries.values()
    }
}
