//! AgentGateway Admission Protocol —— 命令受理决策。
//! 见 `docs/AGENT_GATEWAY_ADMISSION.md` 中协议文档。本模块只承担两件事：
//! 2. `AdmissionPolicy::decide` —— 把 `(Command, current_state)` 映射到 `Decision`
//!    的纯函数。该函数不做副作用、可单元测试覆盖完整真值表。
//! 副作用（写 ledger / 写 scoped cache / cancel 等）由 `AgentGateway::admit`
//! 在拿到 `Decision` 后再执行。
//! ## Phase 0 / 1 / 2 实施进度提示
//! - 当前 Phase 0：仅定义类型 + 纯函数 + 单测，gateway 尚未接入。
//! - Phase 1 会让 `AgentGateway` 调用本模块。
//! - 不会引入 buffer 队列或 cancel-in-flight；`send_message` 在
//!   `Tk` / `Ex` 下走 §4.1「直接插入」语义。

use serde::{Deserialize, Serialize};

use crate::state::states;

/// 对外公开命令的类型枚举。
/// 与 FFI 暴露的命令一一对应；不包含 AI 内部触发的 `appoint` / `dismiss` /
/// `set_focus`。运行时层 `set_default_model` 不依赖会话状态机，因此这里
/// 也不列出。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// 用户发送一条消息（直接插入 ledger 语义）。
    SendMessage {
        content: String,
    },
    /// 暂停当前会话。
    Pause,
    /// 切换会话级推理模型。
    SetConversationModel {
        model: String,
    },
    /// 切换会话级摘要模型（用于 `compact_history`）。
    SetConversationSummaryModel {
        model: String,
    },
    /// 切换会话级语言。
    SetConversationLanguage {
        language: String,
    },
    CompactHistory {
        agent_ids: Vec<String>,
    },
}

impl Command {
    /// 命令的稳定字符串名（写入事件 payload `command` 字段）。
    pub fn name(&self) -> &'static str {
        match self {
            Command::SendMessage { .. } => "send_message",
            Command::Pause => "pause",
            Command::SetConversationModel { .. } => "set_conversation_model",
            Command::SetConversationSummaryModel { .. } => "set_conversation_summary_model",
            Command::SetConversationLanguage { .. } => "set_conversation_language",
            Command::CompactHistory { .. } => "compact_history",
        }
    }
}

/// 受理决策结果。
/// 仅两种结果：`Accepted` / `Rejected`。本协议不引入 buffer / preempt。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Accepted {
        /// `send_message` 在非 `suspended` 状态被插入时记录当时的状态名，
        /// 前端可在气泡上提示「AI 会在下一轮看到」。
        #[serde(skip_serializing_if = "Option::is_none")]
        inserted_during_state: Option<String>,
    },
    Rejected {
        reason: RejectReason,
    },
}

impl Decision {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Decision::Accepted { .. })
    }

    pub fn inserted_during_state(&self) -> Option<&str> {
        match self {
            Decision::Accepted {
                inserted_during_state,
            } => inserted_during_state.as_deref(),
            Decision::Rejected { .. } => None,
        }
    }

    pub fn reject_reason(&self) -> Option<&RejectReason> {
        match self {
            Decision::Accepted { .. } => None,
            Decision::Rejected { reason } => Some(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    AlreadySuspended,
    /// The active conversation is currently compacting history.
    CompactInProgress,
    /// Manual compaction is only valid while the whole conversation is waiting.
    ConversationNotWaiting,
}

impl RejectReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            RejectReason::AlreadySuspended => "already_suspended",
            RejectReason::CompactInProgress => "compact_in_progress",
            RejectReason::ConversationNotWaiting => "conversation_not_waiting",
        }
    }
}

/// 受理策略——纯函数实现，零副作用。
pub struct AdmissionPolicy;

impl AdmissionPolicy {
    /// 根据命令与当前焦点状态机状态做出受理决策。
    /// `current_state` 取自 `AgentRuntime::sm.current_state()`，期望值为
    /// `crate::state::states` 中的常量之一。未知状态视为 `suspended`
    /// 处理（保守 fallback：所有命令都受理）。
    pub fn decide(command: &Command, current_state: &str) -> Decision {
        match command {
            Command::SendMessage { .. } => decide_send_message(current_state),
            Command::Pause => decide_pause(current_state),
            Command::SetConversationModel { .. }
            | Command::SetConversationSummaryModel { .. }
            | Command::SetConversationLanguage { .. } => Decision::Accepted {
                inserted_during_state: None,
            },
            Command::CompactHistory { .. } => {
                if current_state == states::SUSPENDED {
                    Decision::Accepted {
                        inserted_during_state: None,
                    }
                } else {
                    Decision::Rejected {
                        reason: RejectReason::ConversationNotWaiting,
                    }
                }
            }
        }
    }

    pub fn decide_dry(command: &Command, current_state: &str) -> Decision {
        Self::decide(command, current_state)
    }
}

fn decide_send_message(state: &str) -> Decision {
    match state {
        states::SUSPENDED | states::SAYING => Decision::Accepted {
            inserted_during_state: None,
        },
        states::THINKING | states::EXECUTING => Decision::Accepted {
            inserted_during_state: Some(state.to_string()),
        },
        // 未知状态保守按 suspended 处理。
        _ => Decision::Accepted {
            inserted_during_state: None,
        },
    }
}

fn decide_pause(state: &str) -> Decision {
    match state {
        states::SUSPENDED => Decision::Rejected {
            reason: RejectReason::AlreadySuspended,
        },
        _ => Decision::Accepted {
            inserted_during_state: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd_send() -> Command {
        Command::SendMessage {
            content: "hi".into(),
        }
    }

    fn assert_accepted(decision: &Decision, inserted: Option<&str>) {
        match decision {
            Decision::Accepted {
                inserted_during_state,
            } => {
                assert_eq!(inserted_during_state.as_deref(), inserted, "{:?}", decision);
            }
            other => panic!("expected accepted, got {:?}", other),
        }
    }

    fn assert_rejected(decision: &Decision, reason: RejectReason) {
        match decision {
            Decision::Rejected { reason: r } => assert_eq!(r, &reason),
            other => panic!("expected rejected, got {:?}", other),
        }
    }

    // -------- send_message --------

    #[test]
    fn send_message_in_suspended_accepted_no_insert_marker() {
        let d = AdmissionPolicy::decide(&cmd_send(), states::SUSPENDED);
        assert_accepted(&d, None);
    }

    #[test]
    fn send_message_in_saying_accepted_no_insert_marker() {
        let d = AdmissionPolicy::decide(&cmd_send(), states::SAYING);
        assert_accepted(&d, None);
    }

    #[test]
    fn send_message_in_thinking_accepted_with_insert_marker() {
        let d = AdmissionPolicy::decide(&cmd_send(), states::THINKING);
        assert_accepted(&d, Some(states::THINKING));
    }

    #[test]
    fn send_message_in_executing_accepted_with_insert_marker() {
        let d = AdmissionPolicy::decide(&cmd_send(), states::EXECUTING);
        assert_accepted(&d, Some(states::EXECUTING));
    }

    #[test]
    fn send_message_in_unknown_state_falls_back_to_accepted() {
        let d = AdmissionPolicy::decide(&cmd_send(), "weird-state");
        assert_accepted(&d, None);
    }

    // -------- pause --------

    #[test]
    fn pause_in_suspended_rejected_already_suspended() {
        let d = AdmissionPolicy::decide(&Command::Pause, states::SUSPENDED);
        assert_rejected(&d, RejectReason::AlreadySuspended);
    }

    #[test]
    fn pause_in_saying_accepted() {
        let d = AdmissionPolicy::decide(&Command::Pause, states::SAYING);
        assert_accepted(&d, None);
    }

    #[test]
    fn pause_in_thinking_accepted() {
        let d = AdmissionPolicy::decide(&Command::Pause, states::THINKING);
        assert_accepted(&d, None);
    }

    #[test]
    fn pause_in_executing_accepted() {
        let d = AdmissionPolicy::decide(&Command::Pause, states::EXECUTING);
        assert_accepted(&d, None);
    }

    // -------- 配置类命令（任意状态都 accepted，无 insert 标记） --------

    #[test]
    fn config_commands_accepted_in_all_states() {
        let configs = [
            Command::SetConversationModel {
                model: "qwen-max".into(),
            },
            Command::SetConversationSummaryModel {
                model: "qwen-plus".into(),
            },
            Command::SetConversationLanguage {
                language: "zh".into(),
            },
        ];
        for state in [
            states::SUSPENDED,
            states::SAYING,
            states::THINKING,
            states::EXECUTING,
        ] {
            for cmd in &configs {
                let d = AdmissionPolicy::decide(cmd, state);
                assert_accepted(&d, None);
            }
        }
    }

    // -------- compact_history（仅 suspended accepted） --------

    #[test]
    fn compact_history_requires_suspended_state() {
        let cmd = Command::CompactHistory { agent_ids: vec![] };
        assert_accepted(&AdmissionPolicy::decide(&cmd, states::SUSPENDED), None);
        for state in [states::SAYING, states::THINKING, states::EXECUTING] {
            assert_rejected(
                &AdmissionPolicy::decide(&cmd, state),
                RejectReason::ConversationNotWaiting,
            );
        }
    }

    // -------- Command::name 稳定性 --------

    #[test]
    fn command_name_stable_strings() {
        assert_eq!(cmd_send().name(), "send_message");
        assert_eq!(Command::Pause.name(), "pause");
        assert_eq!(
            Command::SetConversationModel { model: "x".into() }.name(),
            "set_conversation_model"
        );
        assert_eq!(
            Command::SetConversationSummaryModel { model: "x".into() }.name(),
            "set_conversation_summary_model"
        );
        assert_eq!(
            Command::SetConversationLanguage {
                language: "zh".into()
            }
            .name(),
            "set_conversation_language"
        );
        assert_eq!(
            Command::CompactHistory { agent_ids: vec![] }.name(),
            "compact_history"
        );
    }

    #[test]
    fn reject_reason_str_stable() {
        assert_eq!(RejectReason::AlreadySuspended.as_str(), "already_suspended");
        assert_eq!(
            RejectReason::CompactInProgress.as_str(),
            "compact_in_progress"
        );
        assert_eq!(
            RejectReason::ConversationNotWaiting.as_str(),
            "conversation_not_waiting"
        );
    }

    #[test]
    fn decision_is_accepted_helper() {
        assert!(AdmissionPolicy::decide_dry(&cmd_send(), states::SUSPENDED).is_accepted());
        assert!(!AdmissionPolicy::decide_dry(&Command::Pause, states::SUSPENDED).is_accepted());
    }
}
