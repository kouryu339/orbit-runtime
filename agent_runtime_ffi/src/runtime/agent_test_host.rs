use super::*;

#[derive(Clone)]
pub(crate) struct AgentTestRuntimeHost {
    pub(super) config: RuntimeConfig,
    pub(super) target_agent: AgentSection,
    pub(super) adversary_cluster: RuntimeAgentCluster,
    pub(super) manager: Arc<ConversationManager>,
    pub(super) export_event_bus: Arc<dyn EventBus>,
    pub(super) state_store: Arc<dyn RuntimeStateStore>,
    pub(super) coordination_backend: Arc<dyn RuntimeCoordinationBackend>,
    pub(super) conversation_owner_leases: Arc<StdMutex<HashMap<String, ConversationOwnerLease>>>,
    pub(super) logs_dir: Option<PathBuf>,
    pub(super) conversation_log_policy: ConversationLogPolicy,
    pub(super) conversation_logs: Arc<StdMutex<HashMap<String, ConversationInstanceMetadata>>>,
}

impl AgentTestRuntimeHost {
    async fn create_cluster_conversation(
        &self,
        spec: &PairConversationSpec,
        cluster: &RuntimeAgentCluster,
        agent_id: &str,
    ) -> Result<(), RuntimeError> {
        let registered_agent = cluster
            .agents
            .iter()
            .find(|agent| agent.id == agent_id)
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "built-in cluster '{}' agent '{}' was not found",
                    cluster.id, agent_id
                ))
            })?;
        let agent = runtime_agent_definition_to_agent_section(registered_agent);
        let skills = active_skill_names(&agent);
        tracing::info!(
            conversation_id = %spec.conversation_id,
            agent_id = %agent.id,
            skills = ?skills,
            "creating agent test conversation"
        );
        let assistant_config = assistant_config_for_runtime(&self.config, &agent);
        let mut options = ConversationOptions::new(spec.conversation_id.clone(), assistant_config);
        options.tenant_id = Some("agent-test-studio".to_string());
        options.user_id = Some(spec.agent_id.clone());
        options.tool_permissions = cluster.permissions.clone();
        if self
            .conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .contains_key(&spec.conversation_id)
        {
            return Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' is already active in this runtime instance",
                spec.conversation_id
            )));
        }

        let runtime_cluster_id = self.config.runtime.cluster_id.clone();
        let lifecycle_cluster_id = cluster.id.clone();
        let lifecycle_cluster_description = cluster.description.clone();
        let runtime_instance_id = self.config.runtime.runtime_instance_id.clone();
        let conversation_id = spec.conversation_id.clone();
        let Some(owner_lease) = acquire_conversation_owner_lease(
            Arc::clone(&self.coordination_backend),
            runtime_cluster_id.clone(),
            conversation_id.clone(),
            runtime_instance_id.clone(),
            LOCAL_LEASE_TTL_MS,
            lease_renew_interval(LOCAL_LEASE_TTL_MS, LOCAL_LEASE_RENEW_INTERVAL_MS),
        )
        .await?
        else {
            return Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' is already owned by another runtime instance",
                conversation_id
            )));
        };

        let result = async {
            let info = self
                .manager
                .create_conversation(options, Arc::clone(&self.export_event_bus))
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            tracing::info!(
                conversation_id = %conversation_id,
                "created agent test conversation runtime"
            );
            if !skills.is_empty() {
                let refs: Vec<&str> = skills.iter().map(String::as_str).collect();
                self.manager
                    .activate_skills(&conversation_id, &refs)
                    .await
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                tracing::info!(
                    conversation_id = %conversation_id,
                    "activated agent test conversation skills"
                );
            }
            if let Some(appendix) = spec.immutable_role_appendix.as_deref() {
                set_immutable_role_appendix(&self.manager, &conversation_id, appendix).await?;
            }
            record_conversation_created(
                Arc::clone(&self.state_store),
                Arc::clone(&self.coordination_backend),
                runtime_cluster_id,
                runtime_instance_id,
                LOCAL_LEASE_TTL_MS,
                info.clone(),
            )
            .await?;
            publish_conversation_created_event(
                Arc::clone(&self.export_event_bus),
                &info,
                &lifecycle_cluster_id,
                &lifecycle_cluster_description,
            )
            .await
        }
        .await;

        if let Err(error) = result {
            let _ = self.manager.close(&conversation_id).await;
            let _ = owner_lease
                .stop_and_release(Arc::clone(&self.coordination_backend))
                .await;
            return Err(error);
        }
        self.conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .insert(conversation_id, owner_lease);
        let log_path = create_conversation_log_path(
            self.logs_dir.as_deref(),
            &lifecycle_cluster_id,
            &spec.conversation_id,
            chrono::Utc::now(),
            &self.conversation_log_policy,
        );
        let metadata = ConversationInstanceMetadata {
            cluster_id: lifecycle_cluster_id,
            cluster_description: lifecycle_cluster_description,
            log_path,
            log_policy: self.conversation_log_policy.clone(),
        };
        append_conversation_log_path(
            metadata.log_path.as_deref(),
            &self.config.runtime.runtime_instance_id,
            &metadata.cluster_id,
            &spec.conversation_id,
            "conversation_created",
            Value::Object(Default::default()),
            metadata.log_policy.max_file_bytes,
        );
        self.conversation_logs
            .lock()
            .map_err(|_| RuntimeError::Internal("conversation log lock poisoned".to_string()))?
            .insert(spec.conversation_id.clone(), metadata);
        Ok(())
    }

    async fn create_business_target_conversation(
        &self,
        spec: &PairConversationSpec,
        agent: &AgentSection,
    ) -> Result<(), RuntimeError> {
        let cluster = RuntimeAgentCluster {
            id: "agent-test-target".to_string(),
            description: "Isolated business target conversation for Agent Test".to_string(),
            focus_agent_id: agent.id.clone(),
            agents: vec![RuntimeAgentDefinition {
                id: agent.id.clone(),
                profile_id: None,
                name: agent.name.clone(),
                role: agent.role.clone(),
                features: agent.features.clone(),
                model_uid: key_store::current().unwrap_or(0),
                retrieval: agent.retrieval.clone(),
                system_prompt_constraints: effective_system_prompt_constraints(
                    agent.frontend_widgets_enabled,
                    &agent.system_prompt_constraints,
                ),
                frontend_widgets_enabled: effective_frontend_widgets_enabled(
                    agent.frontend_widgets_enabled,
                    &agent.system_prompt_constraints,
                ),
            }],
            max_thinking_rounds: 0,
            permissions: ai_assistant::ToolPermissionPolicy::default(),
        };
        self.create_cluster_conversation(spec, &cluster, &agent.id)
            .await
    }

    async fn close(&self, conversation_id: &str) -> Result<(), RuntimeError> {
        let owner_lease = self
            .conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .remove(conversation_id)
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "conversation '{}' is not owned by this runtime instance",
                    conversation_id
                ))
            })?;
        let removed = self
            .manager
            .close(conversation_id)
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        if removed {
            let metadata = self
                .conversation_logs
                .lock()
                .map_err(|_| RuntimeError::Internal("conversation log lock poisoned".to_string()))?
                .remove(conversation_id);
            record_conversation_closed(
                Arc::clone(&self.state_store),
                Arc::clone(&self.coordination_backend),
                self.config.runtime.cluster_id.clone(),
                self.config.runtime.runtime_instance_id.clone(),
                LOCAL_LEASE_TTL_MS,
                conversation_id.to_string(),
            )
            .await?;
            let lifecycle_cluster_id = metadata
                .as_ref()
                .map(|metadata| metadata.cluster_id.as_str())
                .unwrap_or("agent-test");
            let lifecycle_cluster_description = metadata
                .as_ref()
                .map(|metadata| metadata.cluster_description.as_str())
                .unwrap_or("Agent Test conversation");
            publish_conversation_closed_event(
                Arc::clone(&self.export_event_bus),
                conversation_id,
                lifecycle_cluster_id,
                lifecycle_cluster_description,
            )
            .await?;
            append_conversation_log_path(
                metadata
                    .as_ref()
                    .and_then(|metadata| metadata.log_path.as_deref()),
                &self.config.runtime.runtime_instance_id,
                lifecycle_cluster_id,
                conversation_id,
                "conversation_closed",
                Value::Object(Default::default()),
                metadata
                    .as_ref()
                    .map(|metadata| metadata.log_policy.max_file_bytes)
                    .unwrap_or(ConversationLogPolicy::default().max_file_bytes),
            );
        }
        owner_lease
            .stop_and_release(Arc::clone(&self.coordination_backend))
            .await
    }
}

#[async_trait]
impl PairConversationFactory for AgentTestRuntimeHost {
    async fn create_target_conversation(
        &mut self,
        spec: &PairConversationSpec,
    ) -> Result<(), RuntimeError> {
        if spec.agent_id != self.target_agent.id {
            return Err(RuntimeError::InvalidConfig(format!(
                "Agent Test target agent '{}' does not match the resolved Studio target '{}'",
                spec.agent_id, self.target_agent.id
            )));
        }
        let agent = self.target_agent.clone();
        self.create_business_target_conversation(spec, &agent).await
    }

    async fn create_adversary_conversation(
        &mut self,
        spec: &PairConversationSpec,
    ) -> Result<(), RuntimeError> {
        let focus_agent_id = self.adversary_cluster.focus_agent_id.clone();
        let mut spec = spec.clone();
        spec.agent_id = focus_agent_id.clone();
        self.create_cluster_conversation(&spec, &self.adversary_cluster, &focus_agent_id)
            .await
    }

    async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
        self.close(conversation_id).await
    }
}

#[async_trait]
impl PairMessageSender for AgentTestRuntimeHost {
    async fn send_message_with_admission(
        &mut self,
        conversation_id: &str,
        content: &str,
    ) -> Result<bool, RuntimeError> {
        let host = self.clone();
        let conversation_id = conversation_id.to_string();
        let content = content.to_string();
        tokio::spawn(async move {
            if let Err(error) = host
                .send_relay_message_with_admission(&conversation_id, &content)
                .await
            {
                tracing::warn!(
                    conversation_id = %conversation_id,
                    "agent test relay send failed: {error}"
                );
                let _ = host
                    .export_event_bus
                    .publish(BaseEvent::new(
                        crate::agent_test_studio::pair_runtime::RELAY_SEND_FAILED_EVENT,
                        json!({
                            "conversation_id": conversation_id,
                            "failure": error.to_string()
                        }),
                    ))
                    .await;
            }
        });
        Ok(true)
    }
}

impl AgentTestRuntimeHost {
    async fn send_relay_message_with_admission(
        &self,
        conversation_id: &str,
        content: &str,
    ) -> Result<(), RuntimeError> {
        static NEXT_RELAY_COMMAND: AtomicU64 = AtomicU64::new(1);
        let lease_key = format!(
            "runtime:{}:conversation:{}:turn",
            self.config.runtime.cluster_id, conversation_id
        );
        let lease_owner = self.config.runtime.runtime_instance_id.clone();
        let acquired = self
            .coordination_backend
            .acquire_lease(&lease_key, &lease_owner, LOCAL_LEASE_TTL_MS)
            .await?;
        if !acquired {
            return Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' turn lease is held by another runtime",
                conversation_id
            )));
        }
        let sequence = NEXT_RELAY_COMMAND.fetch_add(1, Ordering::Relaxed);
        let admission = self
            .manager
            .send_message_with_admission(
                conversation_id,
                content,
                Some(format!("agent-test-relay-{sequence}")),
            )
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()));
        let release = self
            .coordination_backend
            .release_lease(&lease_key, &lease_owner)
            .await;
        match (admission, release) {
            (Ok(admission), Ok(())) if admission.decision.is_accepted() => Ok(()),
            (Ok(_), Ok(())) => Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' rejected the relay message admission",
                conversation_id
            ))),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(release_error)) => {
                tracing::warn!("release agent test turn lease failed: {}", release_error);
                Err(error)
            }
        }
    }
}

#[async_trait]
impl PairConclusionHost for AgentTestRuntimeHost {
    async fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
        self.close(conversation_id).await
    }

    async fn publish_agent_test_event(
        &mut self,
        event_type: &str,
        payload: Value,
    ) -> Result<(), RuntimeError> {
        self.export_event_bus
            .publish(BaseEvent::new(event_type, payload))
            .await
            .map_err(|error| RuntimeError::Internal(error.to_string()))
    }
}
