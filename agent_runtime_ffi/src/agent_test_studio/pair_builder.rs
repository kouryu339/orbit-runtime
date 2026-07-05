use async_trait::async_trait;

use super::controller::{AdversaryPair, AdversaryPairStatus, AgentTestController};
use super::pair_runtime::AdversaryPairRuntime;
use super::role_contract::AdversaryPersona;
use crate::runtime::RuntimeError;

#[derive(Debug, Clone)]
pub struct AdversaryPairBuildRequest {
    pub pair_id: String,
    pub target_agent_id: String,
    pub persona: AdversaryPersona,
    pub initial_message: String,
}

#[derive(Debug, Clone)]
pub struct PairConversationSpec {
    pub conversation_id: String,
    pub agent_id: String,
    pub immutable_role_appendix: Option<String>,
}

#[async_trait]
pub trait PairConversationFactory: Send {
    async fn create_target_conversation(
        &mut self,
        spec: &PairConversationSpec,
    ) -> Result<(), RuntimeError>;

    async fn create_adversary_conversation(
        &mut self,
        spec: &PairConversationSpec,
    ) -> Result<(), RuntimeError>;

    async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError>;
}

pub struct AdversaryPairBuilder<'a, F> {
    controller: &'a AgentTestController,
    factory: &'a mut F,
}

impl<'a, F> AdversaryPairBuilder<'a, F>
where
    F: PairConversationFactory,
{
    pub fn new(controller: &'a AgentTestController, factory: &'a mut F) -> Self {
        Self {
            controller,
            factory,
        }
    }

    pub async fn build(
        &mut self,
        request: AdversaryPairBuildRequest,
    ) -> Result<AdversaryPairRuntime, RuntimeError> {
        validate_request(&request)?;

        let pair_id = request.pair_id.trim().to_string();
        let initial_message = request.initial_message.trim().to_string();
        let target_conversation_id = format!("agent-test:{pair_id}:target");
        let adversary_conversation_id = format!("agent-test:{pair_id}:adversary");
        let role_appendix = format!(
            "# Immutable Adversary Persona\n\n{}",
            serde_json::to_string_pretty(&request.persona)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?
        );
        self.controller
            .reserve_pair(AdversaryPair {
                pair_id: pair_id.clone(),
                adversary_conversation_id: adversary_conversation_id.clone(),
                target_conversation_id: target_conversation_id.clone(),
                persona: request.persona,
                status: AdversaryPairStatus::Starting,
            })
            .await
            .map_err(RuntimeError::InvalidConfig)?;

        let target_spec = PairConversationSpec {
            conversation_id: target_conversation_id.clone(),
            agent_id: request.target_agent_id,
            immutable_role_appendix: None,
        };
        tracing::info!(pair_id = %pair_id, "creating agent test target conversation");
        if let Err(error) = self.factory.create_target_conversation(&target_spec).await {
            self.controller.release_pair_reservation(&pair_id).await;
            return Err(error);
        }
        tracing::info!(pair_id = %pair_id, "created agent test target conversation");

        let adversary_spec = PairConversationSpec {
            conversation_id: adversary_conversation_id.clone(),
            agent_id: format!("agent-test-adversary:{pair_id}"),
            immutable_role_appendix: Some(role_appendix),
        };
        tracing::info!(pair_id = %pair_id, "creating agent test adversary conversation");
        if let Err(error) = self
            .factory
            .create_adversary_conversation(&adversary_spec)
            .await
        {
            let rollback_error = self
                .factory
                .close_conversation(&target_conversation_id)
                .await
                .err();
            self.controller.release_pair_reservation(&pair_id).await;
            return match rollback_error {
                Some(rollback_error) => Err(RuntimeError::Internal(format!(
                    "{error}; rollback target conversation failed: {rollback_error}"
                ))),
                None => Err(error),
            };
        }
        tracing::info!(pair_id = %pair_id, "created agent test adversary conversation");

        if let Err(error) = self.controller.activate_pair(&pair_id).await {
            let _ = self
                .factory
                .close_conversation(&adversary_conversation_id)
                .await;
            let _ = self
                .factory
                .close_conversation(&target_conversation_id)
                .await;
            self.controller.release_pair_reservation(&pair_id).await;
            return Err(RuntimeError::Internal(error));
        }

        Ok(AdversaryPairRuntime {
            pair_id,
            adversary_conversation_id,
            target_conversation_id,
            initial_message,
            status: AdversaryPairStatus::Running,
        })
    }
}

fn validate_request(request: &AdversaryPairBuildRequest) -> Result<(), RuntimeError> {
    if request.pair_id.trim().is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "pair_id must not be empty".to_string(),
        ));
    }
    if request.target_agent_id.trim().is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "target_agent_id must not be empty".to_string(),
        ));
    }
    if request.initial_message.trim().is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "initial_message must not be empty".to_string(),
        ));
    }
    if request.persona.identity.trim().is_empty() || request.persona.goal.trim().is_empty() {
        return Err(RuntimeError::InvalidConfig(
            "adversary persona identity and goal must not be empty".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeConversationFactory {
        target_specs: Vec<PairConversationSpec>,
        adversary_specs: Vec<PairConversationSpec>,
        closed: Vec<String>,
        fail_target: bool,
        fail_adversary: bool,
    }

    #[async_trait]
    impl PairConversationFactory for FakeConversationFactory {
        async fn create_target_conversation(
            &mut self,
            spec: &PairConversationSpec,
        ) -> Result<(), RuntimeError> {
            self.target_specs.push(spec.clone());
            if self.fail_target {
                Err(RuntimeError::Internal(
                    "target conversation failed".to_string(),
                ))
            } else {
                Ok(())
            }
        }

        async fn create_adversary_conversation(
            &mut self,
            spec: &PairConversationSpec,
        ) -> Result<(), RuntimeError> {
            self.adversary_specs.push(spec.clone());
            if self.fail_adversary {
                Err(RuntimeError::Internal(
                    "adversary conversation failed".to_string(),
                ))
            } else {
                Ok(())
            }
        }

        async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
            self.closed.push(conversation_id.to_string());
            Ok(())
        }
    }

    fn request(pair_id: &str) -> AdversaryPairBuildRequest {
        AdversaryPairBuildRequest {
            pair_id: pair_id.to_string(),
            target_agent_id: "target-agent".to_string(),
            persona: AdversaryPersona {
                identity: "skeptical buyer".to_string(),
                personality: "impatient".to_string(),
                background: "received a leaking bottle".to_string(),
                goal: "obtain an after-sales resolution".to_string(),
                strategy: "reveal evidence only when asked".to_string(),
                hidden_facts: vec!["has a photo".to_string()],
                boundaries: vec!["never mention testing".to_string()],
            },
            initial_message: "The bottle leaked. Please help me.".to_string(),
        }
    }

    #[tokio::test]
    async fn builds_isolated_target_and_adversary_conversations() {
        let controller = AgentTestController::new();
        let mut factory = FakeConversationFactory::default();
        let pair = AdversaryPairBuilder::new(&controller, &mut factory)
            .build(request("pair-001"))
            .await
            .unwrap();

        assert_eq!(pair.status, AdversaryPairStatus::Running);
        assert_eq!(pair.target_conversation_id, "agent-test:pair-001:target");
        assert_eq!(
            pair.adversary_conversation_id,
            "agent-test:pair-001:adversary"
        );
        assert_ne!(pair.target_conversation_id, pair.adversary_conversation_id);
        assert_eq!(pair.initial_message, "The bottle leaked. Please help me.");
        assert_eq!(factory.target_specs[0].agent_id, "target-agent");
        let appendix = factory.adversary_specs[0]
            .immutable_role_appendix
            .as_deref()
            .unwrap();
        assert_eq!(appendix.matches("\"identity\"").count(), 1);
        assert!(appendix.contains("skeptical buyer"));
        assert!(factory.closed.is_empty());
        assert_eq!(
            controller.snapshot().await[0].status,
            AdversaryPairStatus::Running
        );
    }

    #[tokio::test]
    async fn rolls_back_target_when_adversary_creation_fails() {
        let controller = AgentTestController::new();
        let mut factory = FakeConversationFactory {
            fail_adversary: true,
            ..Default::default()
        };
        let error = AdversaryPairBuilder::new(&controller, &mut factory)
            .build(request("pair-rollback"))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("adversary conversation failed"));
        assert_eq!(
            factory.closed,
            vec!["agent-test:pair-rollback:target".to_string()]
        );
        assert!(controller.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn rejects_duplicate_pair_before_creating_more_conversations() {
        let controller = AgentTestController::new();
        let mut factory = FakeConversationFactory::default();
        AdversaryPairBuilder::new(&controller, &mut factory)
            .build(request("pair-duplicate"))
            .await
            .unwrap();

        let error = AdversaryPairBuilder::new(&controller, &mut factory)
            .build(request("pair-duplicate"))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("already exists"));
        assert_eq!(factory.target_specs.len(), 1);
        assert_eq!(factory.adversary_specs.len(), 1);
    }

    #[tokio::test]
    async fn terminal_pair_cannot_be_rebuilt() {
        let controller = AgentTestController::new();
        let mut factory = FakeConversationFactory::default();
        AdversaryPairBuilder::new(&controller, &mut factory)
            .build(request("pair-terminal"))
            .await
            .unwrap();
        controller
            .mark_terminal("pair-terminal", AdversaryPairStatus::Destroyed)
            .await
            .unwrap();

        let error = AdversaryPairBuilder::new(&controller, &mut factory)
            .build(request("pair-terminal"))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("terminal"));
    }
}
