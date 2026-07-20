//! AI助手事件定义
//! ## 事件流
//! ```text
//! thinking：LLM 完成决策  → 发布 ai:thinking-done
//! executing：工具开始执行 → 发布 ai:tool-start
//! executing：工具执行完毕 → 发布 ai:tool-end
//! ```

use crate::gateway::GatewayMessage;
use crate::ledger::LedgerRecord;
use crate::ledger::{LedgerMessageMeta, LedgerRole};
use crate::persistence::DisplayMeta;
use serde::{Deserialize, Serialize};

// ============================================================================
// 事件类型常量
// ============================================================================

/// 事件类型常量模块
pub mod types {
    /// Framework view event: render this message list in the single chat window.
    pub const MESSAGES_CHANGED: &str = "ai:messages-changed";
    /// Canonical ledger record appended. Routers decide projection/persistence/forwarding.
    pub const LEDGER_RECORD_APPENDED: &str = "ai:ledger-record-appended";
    /// Stable export event: append-only ledger replication for hosts.
    pub const CONVERSATION_LEDGER_DELTA: &str = "conversation.ledger_delta";
    /// Stable export event: internal runtime state replication for hosts.
    pub const CONVERSATION_STATE_DELTA: &str = "conversation.state_delta";
    /// State/runtime produced a message fact. The gateway router decides where it is written/routed.
    pub const AGENT_MESSAGE_PRODUCED: &str = "agent:message-produced";
    /// User/runtime requested a pause. The gateway router records the fact and applies cancellation.
    pub const AGENT_PAUSE_REQUESTED: &str = "agent:pause-requested";
    /// Runtime requested appointing focus to another Agent.
    pub const AGENT_APPOINT_REQUESTED: &str = "agent:appoint-requested";
    /// Runtime submitted an Agent report to another Agent.
    pub const AGENT_REPORT_SUBMITTED: &str = "agent:report-submitted";
    /// Runtime created a conversation-scoped delegated background task.
    pub const AGENT_TASK_CREATED: &str = "agent-task:created";
    /// Runtime assigned a delegated background task to an Agent instance.
    pub const AGENT_TASK_ASSIGNED: &str = "agent-task:assigned";
    /// Background Agent reported a delegated task result.
    pub const AGENT_TASK_REPORTED: &str = "agent-task:reported";
    /// Delegated task reached a terminal state and was routed to its delegator.
    pub const AGENT_TASK_COMPLETED: &str = "agent-task:completed";
    /// Host-owned dynamic snapshot was injected for an agent.
    pub const AGENT_DYNAMIC_SNAPSHOT_SET: &str = "agent:dynamic-snapshot-set";
    /// Agent-local skill/tool state changed.
    pub const AGENT_SKILLS_CHANGED: &str = "agent:skills-changed";
    /// Framework view event: current focus UI capability snapshot changed.
    pub const FOCUS_STATUS_CHANGED: &str = "ai:focus-status-changed";
    /// Internal notification that the conversation-level runtime phase changed.
    pub const CONVERSATION_STATE_CHANGED: &str = "conversation:state-changed";

    /// thinking 状态完成 LLM 调用，AI 做出决策
    pub const THINKING_DONE: &str = "ai:thinking-done";
    /// 工具开始执行
    pub const TOOL_START: &str = "ai:tool-start";
    /// 工具执行完毕
    pub const TOOL_END: &str = "ai:tool-end";
    /// A tool invocation is waiting for a host/user permission decision.
    pub const TOOL_PERMISSION_REQUESTED: &str = "tool:permission-requested";
    /// A pending tool permission request reached a terminal decision.
    pub const TOOL_PERMISSION_RESOLVED: &str = "tool:permission-resolved";
    /// 请求前端打开独立窗口（录制模式启动等场景）
    pub const OPEN_WINDOW: &str = "ui:open-window";
    /// 子 Agent 已上线，对话焦点切换到此 Agent
    pub const AGENT_ACTIVE: &str = "agent:active";
    /// 子 Agent 已完成/取消，对话焦点归还 Boss
    pub const AGENT_COMPLETED: &str = "agent:completed";
    /// 当前焦点 Agent 发生变化
    pub const AGENT_FOCUS_CHANGED: &str = "agent:focus-changed";
    /// Agent 已进入暂停
    pub const AGENT_SUSPENDED: &str = "agent:suspended";
    /// Agent 从暂停恢复
    pub const AGENT_RESUMED: &str = "agent:resumed";
    /// Agent 被取消或销毁
    pub const AGENT_CANCELED: &str = "agent:canceled";
    /// 流式重置：通知前端丢弃当前 running 的 assistant 气泡（FC 重试时使用）
    pub const STREAM_RESET: &str = "ai:stream-reset";
    /// 草稿已锁定：焦点 Agent 进入 thinking / executing，前端应进入只读模式
    pub const DRAFT_LOCKED: &str = "draft:locked";
    pub const DRAFT_UNLOCKED: &str = "draft:unlocked";
    /// 用户主动中断 AI 请求（前端可渲染"已停止"气泡）
    /// ## 触发时机
    /// thinking 状态的 LLM HTTP 请求被 `CancellationToken` 中断后，
    /// 在走暂停路径（`consume_pause_if_requested`）之前发布此事件。
    /// ## 载荷
    /// 空 JSON `{}`，无附加数据。
    /// ## 对话历史影响
    /// **不修改对话历史**。与 Claude Code 行为一致：
    /// - 中断消息仅用于前端显示，不写入后端 conversation
    /// - 下一轮 LLM 调用时模型不可见此消息
    /// - 用户重新发消息后，AI 从用户最后一条消息开始重新思考
    pub const INTERRUPTED: &str = "ai:interrupted";
    /// AI 本轮处理完毕，推送 `saying` 稳态产生的最终回复。
    /// 为兼容现有前端通道，事件名仍保持 `ai:asking`。
    pub const ASKING: &str = "ai:asking";
    /// `saying` 稳态事件的语义别名，当前与 `ASKING` 使用同一事件名。
    pub const SAYING: &str = ASKING;
    /// 本轮处理完毕通知（配合 `ai:asking` 一起发出）
    /// 前端收到此事件后关闭 loading 状态。载荷见 [`TurnDonePayload`]。
    pub const TURN_DONE: &str = "ai:turn-done";
    /// 新一轮开始通知（thinking on_enter 在 bump turn_id 后立即发布）
    /// 载荷见 [`TurnStartPayload`]。
    pub const TURN_START: &str = "ai:turn-start";
    /// 草稿快照已刷新 —— `ui:snapshot-updated`
    /// 由 workflows crate 的 `refresh_world_snapshot` 触发：
    /// 任何 `draft_*` 命令（契约 CRUD / WriteScript 等）成功后，
    /// 都会广播新版本的快照文本给前端。
    /// 载荷见 [`SnapshotUpdatedPayload`]。
    pub const SNAPSHOT_UPDATED: &str = "ui:snapshot-updated";

    /// LLM 网关返回致命错误（重试用尽 / 401 / 400 等）—— `ai:llm-error`
    /// thinking 状态在收到 `ApiError::Fatal` 时发布，前端据此渲染
    /// 友好的错误气泡（带建议动作 + 是否可重试）。
    /// 载荷见 [`LlmErrorPayload`]。
    pub const LLM_ERROR: &str = "ai:llm-error";
    /// Conversation-scoped LLM token usage telemetry.
    pub const LLM_USAGE: &str = "ai:llm-usage";
    /// 新计划已写入 —— `plan:written`
    /// `PlanWrite` 工具建立新计划后发布。前端据此显示计划面板 / 进度条。
    /// 载荷见 [`PlanChangedPayload`]。
    pub const PLAN_WRITTEN: &str = "plan:written";

    /// 计划已更新 —— `plan:updated`
    /// `PlanUpdate` 工具覆盖当前计划内容后发布。
    /// 前端应刷新计划面板并可在节点完成时做高亮动画。
    /// 载荷见 [`PlanChangedPayload`]。
    pub const PLAN_UPDATED: &str = "plan:updated";

    /// 计划已完成 —— `plan:finished`
    /// `PlanFinish` 工具把当前计划置为 `finished` 后发布。
    /// 前端可折叠 / 归档计划面板，或显示"任务完成"提示。
    /// 载荷见 [`PlanChangedPayload`]。
    pub const PLAN_FINISHED: &str = "plan:finished";

    // ------------------------------------------------------------------
    // AgentGateway Admission Protocol（详见 docs/AGENT_GATEWAY_ADMISSION.md）
    // ------------------------------------------------------------------

    /// 网关受理决策事件 —— `gateway:command-admitted`
    /// 任何对外命令进入 `AgentGateway::admit` 时都会发布此事件，载荷包含
    /// `command_id` / `command` / `decision` / `current_state` 等字段，
    /// 前端可据此立即更新按钮状态、记录审计轨迹。
    pub const GATEWAY_COMMAND_ADMITTED: &str = "gateway:command-admitted";

    /// `compact_history` 命令对某个 agent 因末尾 20 条已含 summary 而幂等跳过时发布。
    pub const GATEWAY_COMPACT_SKIPPED: &str = "gateway:compact-skipped";

    pub const GATEWAY_COMPACT_DONE: &str = "gateway:compact-done";

    pub const GATEWAY_COMPACT_FAILED: &str = "gateway:compact-failed";

    /// 前端状态快照 —— `frontend:state_snapshot`
    /// 由 `AgentGateway` 监听 conversation-scoped 内部事件后重算并发布。
    /// FFI v3 只需要转发此单一前端状态事件。
    pub const FRONTEND_STATE_SNAPSHOT: &str = "frontend:state_snapshot";
}

// ============================================================================
// 事件载荷结构体
// ============================================================================

/// thinking 完成事件载荷 —— `ai:thinking-done`
/// 前端收到此事件后，追加一条 `thinking_step` 类型的消息气泡。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagesChangedPayload {
    pub agent_id: String,
    pub agent_name: String,
    pub mode: String,
    pub messages: Vec<GatewayMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerRecordAppendedPayload {
    pub record: LedgerRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessageProducedPayload {
    pub conversation_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub role: LedgerRole,
    pub content: String,
    #[serde(default)]
    pub metadata: LedgerMessageMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPauseRequestedPayload {
    pub agent_id: String,
    pub agent_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAppointRequestedPayload {
    pub from_agent_id: String,
    pub from_agent_name: String,
    pub target: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReportSubmittedPayload {
    pub from_agent_id: String,
    pub from_agent_name: String,
    pub target: String,
    pub report_type: String,
    pub report: String,
    #[serde(default)]
    pub handoff: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskCreatedPayload {
    pub task_id: String,
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub acceptance: Vec<String>,
    pub delegator_agent_id: String,
    pub delegator_agent_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskAssignedPayload {
    pub task_id: String,
    pub assignee_agent_id: String,
    pub assignee_agent_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskReportedPayload {
    pub task_id: String,
    pub reporter_agent_id: String,
    pub reporter_agent_name: String,
    pub report_type: String,
    pub summary: String,
    #[serde(default)]
    pub result: serde_json::Value,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusStatusPayload {
    pub agent_id: String,
    pub agent_name: String,
    pub focused_agent_id: String,
    pub status: UiSnapshotStatus,
    pub interaction: UiSnapshotInteraction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSnapshotStatus {
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSnapshotInteraction {
    pub input_enabled: bool,
    pub send_enabled: bool,
    pub pause_visible: bool,
    pub pause_enabled: bool,
    pub pause_label: String,
    pub busy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingDonePayload {
    /// AI 的推理过程文本（来自 `reasoning` 字段）
    pub reasoning: Option<String>,
    /// 决策类型：`"executing"` / `"asking"` / `"result"`
    pub decision: String,
    /// 待执行工具命令列表（`decision == "executing"` 时有值）
    pub tools: Vec<String>,
    /// 所属 turn id（用于前端过滤过期事件）
    #[serde(default)]
    pub turn_id: u64,
}

/// 工具开始执行事件载荷 —— `ai:tool-start`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStartPayload {
    pub name: String,
    /// 完整命令字符串（如 `"CallBaiduOcr --image_url http://…"`）
    pub command: String,
    /// 所属 turn id（用于前端过滤过期事件）
    #[serde(default)]
    pub turn_id: u64,
}

/// 工具执行完毕事件载荷 —— `ai:tool-end`
/// 前端收到此事件后，追加一条 `tool_step` 类型的消息气泡（默认折叠）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEndPayload {
    pub name: String,
    /// 完整命令字符串
    pub command: String,
    /// 执行是否成功
    pub success: bool,
    /// 展示给用户的结果摘要（来自 `ToolResult.to_ai`）
    pub result: String,
    /// 所属 turn id（用于前端过滤过期事件）
    #[serde(default)]
    pub turn_id: u64,
}

/// 流式 chunk 事件载荷 —— `ai:stream`
/// boss_loop 在每个 LLM delta 到来时 emit，前端拼接到当前 assistant 气泡。
/// `done = true` 表示本轮流式结束（由 `ai:turn-done` 同步，此字段目前始终为 false）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunkPayload {
    pub content: String,
    pub done: bool,
    /// 所属 turn id（用于前端过滤过期事件）
    #[serde(default)]
    pub turn_id: u64,
}

/// 打开前端窗口事件载荷 —— `ui:open-window`
/// TauriEventForwarder 转发给前端，前端据此创建独立 Tauri 窗口。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenWindowPayload {
    /// 窗口类型标识（如 `"workflow-editor"`）
    pub window_type: String,
    /// 可选参数（JSON 格式，窗口组件自行解析）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// 子 Agent 上线事件载荷 —— `agent:active`
/// CreateAgentSystem spawn Interactive Agent 后发布，
/// 前端据此切换对话焦点（高亮子 Agent 标签 / 显示切换提示）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentActivePayload {
    pub agent_id: String,
    pub agent_name: String,
}

/// 子 Agent 完成事件载荷 —— `agent:completed`
/// Interactive Agent 循环结束（完成/失败/取消）后发布，
/// 前端据此恢复 Boss 对话焦点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCompletedPayload {
    pub agent_id: String,
    pub agent_name: String,
}

/// AI 最终回复事件载荷 —— `ai:asking` / `ai:saying`（语义别名）
/// 对应 `PENDING_RESPONSE`（`saying` 状态写入的展示文本）。
/// 说明：实时 `saying` 事件不再携带 `view`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskingPayload {
    /// 最终回复文本
    pub content: String,
    /// 所属 turn id（用于前端过滤过期事件）
    #[serde(default)]
    pub turn_id: u64,
}

pub type SayingPayload = AskingPayload;

/// 用户主动中断事件载荷 —— `ai:interrupted`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptedPayload {
    /// 被中断的 turn id（前端据此判断是否仍应渲染"已停止"气泡）
    #[serde(default)]
    pub turn_id: u64,
}

/// 新一轮开始事件载荷 —— `ai:turn-start`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnStartPayload {
    pub turn_id: u64,
}

/// 本轮结束事件载荷 —— `ai:turn-done`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnDonePayload {
    #[serde(default)]
    pub turn_id: u64,
}

/// 草稿锁定状态变更事件载荷 —— `draft:locked` / `draft:unlocked`
/// 焦点 Agent 进入/离开 thinking / executing 时发布，
/// 后端 Tauri command 层通过进程级 [`DRAFT_LOCKED`] AtomicBool 检查。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftLockPayload {
    /// 产生锁定/解锁的 Agent ID（Boss 时为空字符串 ""）
    pub agent_id: String,
    /// 触发变更的状态名（"thinking" / "executing"）
    pub state: String,
}

/// 快照刷新事件载荷 —— `ui:snapshot-updated`
/// 每次草稿变更后由 `workflows::snapshot::refresh_world_snapshot` 触发。
/// 前端 recorder agent 视图可直接用 `snapshot_text` 替换当前 `<current_draft>` 区块；
/// 需做乐观锁的调用方可以保留 `version` 作为 `WriteScript(base_version=…)` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotUpdatedPayload {
    /// 完整快照文本（CONTRACT + SCRIPT 两块）
    pub snapshot_text: String,
    /// 单调递增的版本号
    pub version: u64,
}

/// LLM 致命错误事件载荷 —— `ai:llm-error`
/// thinking 状态收到 `llm_gateway::ApiError::Fatal` 时发布，
/// 前端据此渲染错误气泡（红/黄底 + 建议 + 重试按钮）。
/// 字段直接来自 `ApiError::user_message` / `kind_str` / `suggestion` / `retryable_by_user`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmErrorPayload {
    /// 所属会话 id
    #[serde(default)]
    pub conversation_id: String,
    /// 所属 agent id
    #[serde(default)]
    pub agent_id: String,
    /// 错误类型字符串（如 `"rate_limit_exhausted"` / `"auth_failed"` / `"context_too_long"`）
    pub kind: String,
    /// 给用户看的中文友好提醒
    pub message: String,
    /// 建议动作（可选）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// 用户手动重试是否值得（前端"重试"按钮 enable/disable）
    pub can_retry: bool,
    /// 网关已尝试次数（重试用尽时大于 1）
    #[serde(default)]
    pub attempts: u32,
    /// HTTP 状态码（若有）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// 所属 turn id（前端过滤过期事件）
    #[serde(default)]
    pub turn_id: u64,
}

/// 计划变更事件载荷 —— `plan:written` / `plan:updated` / `plan:finished`
/// 三个计划事件共用同一载荷结构，前端可按事件类型区分动画与处理方式：
/// - `plan:written`：新建计划面板
/// - `plan:updated`：刷新面板并对变化段做高亮
/// - `plan:finished`：关闭 / 归档面板
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmUsagePayload {
    pub conversation_id: String,
    pub agent_id: String,
    pub turn_id: u64,
    pub model_uid: u32,
    pub model: String,
    pub provider_uid: u32,
    pub input_tokens: u32,
    pub cached_input_tokens: u32,
    pub uncached_input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    pub total_billable_tokens: u32,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanChangedPayload {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub agent_name: String,
    /// 计划标题
    pub title: String,
    /// 计划摘要
    #[serde(default)]
    pub summary: String,
    /// 计划正文（Markdown）
    pub content: String,
    /// 状态：`"active"` / `"finished"`
    pub status: String,
    /// 最近一次更新时间（RFC3339）
    pub updated_at: String,
}

// ============================================================================
// 进程级草稿锁标志
// ============================================================================

/// 进程级草稿锁标志
/// 由状态机的 on_enter / on_exit 通过 [`set_draft_locked`] 更新，
/// 由 Tauri command 层通过 [`is_draft_locked`] 读取。
/// 无需持有任何状态机引用，避免轮询或跨层级锁争用。
static DRAFT_LOCKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 设置草稿锁状态（由状态机 on_enter / on_exit 调用）
pub fn set_draft_locked(locked: bool) {
    DRAFT_LOCKED.store(locked, std::sync::atomic::Ordering::Release);
}

/// 读取草稿锁状态（由 Tauri command 层调用，无 await，无阻塞）
pub fn is_draft_locked() -> bool {
    DRAFT_LOCKED.load(std::sync::atomic::Ordering::Acquire)
}
