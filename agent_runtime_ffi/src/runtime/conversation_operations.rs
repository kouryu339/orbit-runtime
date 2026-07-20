use super::*;

impl RuntimeFacade {
    pub fn set_ai_auth_context(&mut self, context_json: &str) -> Result<(), RuntimeError> {
        let headers = parse_ai_auth_context_headers(context_json)?;
        self.ai_auth_context_headers = headers.clone();
        if let Some(manager) = self.conversation_manager.clone() {
            self.rt
                .block_on(manager.set_runtime_llm_request_headers(headers));
        }
        Ok(())
    }

    pub fn create_conversation(
        &mut self,
        options_json: &str,
    ) -> Result<ConversationInfo, RuntimeError> {
        let agent = self.config.default_agent().clone();
        let skills = self.active_skill_names_for_agent(&agent);
        self.create_conversation_with_agent_and_skills(options_json, &agent, skills)
    }

    pub fn spawn_conversation(
        &mut self,
        spawn_json: &str,
    ) -> Result<ConversationInfo, RuntimeError> {
        let request = parse_conversation_spawn_request(spawn_json)?;
        let cluster = self
            .registries
            .agent_clusters
            .get(&request.cluster_id)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "agent cluster '{}' is not registered",
                    request.cluster_id
                ))
            })?;
        let focus_agent = cluster
            .agents
            .iter()
            .find(|agent| agent.id == cluster.focus_agent_id)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "agent cluster '{}' focus_agent_id '{}' was not found",
                    cluster.id, cluster.focus_agent_id
                ))
            })?;
        let agent = runtime_agent_definition_to_agent_section(&focus_agent);
        let skills = active_skill_names(&agent);
        let mut additional_agents = Vec::new();
        for registered_agent in cluster
            .agents
            .iter()
            .filter(|registered_agent| registered_agent.id != cluster.focus_agent_id)
        {
            let section = runtime_agent_definition_to_agent_section(registered_agent);
            additional_agents.push(ConversationAgentInit {
                config: self.assistant_config_for_agent(&section),
                skills: active_skill_names(&section),
                max_thinking_rounds: cluster.max_thinking_rounds,
            });
        }
        let init = ConversationInitPlan {
            model_uid: (focus_agent.model_uid != 0).then_some(focus_agent.model_uid),
            max_thinking_rounds: Some(cluster.max_thinking_rounds),
            tool_host_context: request.tool_host_context.clone(),
            tool_permissions: Some(match &request.permissions {
                Some(permissions) => permissions.apply_to(&cluster.permissions),
                None => cluster.permissions.clone(),
            }),
            lifecycle_cluster_id: Some(cluster.id.clone()),
            lifecycle_cluster_description: Some(cluster.description.clone()),
            additional_agents,
            ..ConversationInitPlan::default()
        };
        if !self.started {
            return Err(RuntimeError::NotStarted);
        }
        self.create_conversation_from_parts(
            ConversationOptionsInput::default(),
            &agent,
            skills,
            init,
        )
    }

    pub fn spawn_conversation_from_snapshot(
        &mut self,
        spawn_json: &str,
        snapshot_json: &str,
    ) -> Result<ConversationInfo, RuntimeError> {
        let snapshot = parse_conversation_snapshot(snapshot_json)?;
        let info = self.spawn_conversation(spawn_json)?;
        let state_deltas = snapshot_state_deltas(&snapshot);
        let recovery =
            match self.replace_conversation_ledger(&info.conversation_id, snapshot.ledger) {
                Ok(recovery) => recovery,
                Err(error) => {
                    let _ = self.close_conversation(&info.conversation_id);
                    return Err(error);
                }
            };
        if let Err(error) =
            self.apply_conversation_state_deltas(&info.conversation_id, state_deltas)
        {
            let _ = self.close_conversation(&info.conversation_id);
            return Err(error);
        }
        if let Err(error) = self.apply_conversation_recovery(&info.conversation_id, recovery) {
            let _ = self.close_conversation(&info.conversation_id);
            return Err(error);
        }
        Ok(info)
    }

    #[allow(dead_code)]
    pub(crate) fn registered_cluster_description(&self, cluster_id: &str) -> Option<&str> {
        self.registries
            .agent_clusters
            .get(cluster_id)
            .map(|cluster| cluster.description.as_str())
    }

    fn create_conversation_with_agent_and_skills(
        &mut self,
        options_json: &str,
        agent: &AgentSection,
        skills: Vec<String>,
    ) -> Result<ConversationInfo, RuntimeError> {
        let options_input = parse_conversation_options(options_json)?;
        self.create_conversation_from_parts(
            options_input,
            agent,
            skills,
            ConversationInitPlan::default(),
        )
    }

    pub(super) fn create_internal_conversation(
        &mut self,
        options_json: &str,
        agent: &AgentSection,
        skills: Vec<String>,
        cluster_id: &str,
        cluster_description: &str,
    ) -> Result<ConversationInfo, RuntimeError> {
        let options_input = parse_conversation_options(options_json)?;
        self.create_conversation_from_parts(
            options_input,
            agent,
            skills,
            ConversationInitPlan {
                lifecycle_cluster_id: Some(cluster_id.to_string()),
                lifecycle_cluster_description: Some(cluster_description.to_string()),
                ..ConversationInitPlan::default()
            },
        )
    }

    pub(super) fn create_conversation_from_cluster_config(
        &mut self,
        options_json: &str,
        cluster: &RuntimeAgentCluster,
    ) -> Result<ConversationInfo, RuntimeError> {
        let focus_agent = cluster
            .agents
            .iter()
            .find(|agent| agent.id == cluster.focus_agent_id)
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "built-in cluster '{}' focus agent '{}' was not found",
                    cluster.id, cluster.focus_agent_id
                ))
            })?;
        let agent = runtime_agent_definition_to_agent_section(focus_agent);
        let options_input = parse_conversation_options(options_json)?;
        self.create_conversation_from_parts(
            options_input,
            &agent,
            active_skill_names(&agent),
            ConversationInitPlan {
                model_uid: (focus_agent.model_uid != 0).then_some(focus_agent.model_uid),
                max_thinking_rounds: Some(cluster.max_thinking_rounds),
                tool_permissions: Some(cluster.permissions.clone()),
                lifecycle_cluster_id: Some(cluster.id.clone()),
                lifecycle_cluster_description: Some(cluster.description.clone()),
                additional_agents: Vec::new(),
                ..ConversationInitPlan::default()
            },
        )
    }

    fn create_conversation_from_parts(
        &mut self,
        options_input: ConversationOptionsInput,
        agent: &AgentSection,
        skills: Vec<String>,
        init: ConversationInitPlan,
    ) -> Result<ConversationInfo, RuntimeError> {
        let manager = self.manager()?;
        let assistant_config = self.assistant_config_for_agent(agent);
        let mut init = init;
        if init.agent_profiles.is_empty() {
            if let Some(resources) = &self.registries.resources {
                init.agent_profiles = resources.agent_profiles.clone();
            }
        }
        let mut options = match options_input.conversation_id {
            Some(id) if !id.trim().is_empty() => ConversationOptions::new(id, assistant_config),
            _ => ConversationOptions::from_config(assistant_config),
        };
        options.tenant_id = options_input
            .tenant_id
            .filter(|value| !value.trim().is_empty());
        options.user_id = options_input
            .user_id
            .filter(|value| !value.trim().is_empty());
        options.llm_request_headers = options_input.llm_request_headers.unwrap_or_default();
        options.allow_insecure_llm_request_headers =
            options_input.allow_insecure_llm_request_headers;
        if let Some(policy) = init.tool_permissions.clone() {
            options.tool_permissions = policy;
        }
        let requested_conversation_id = options.conversation_id.clone();
        if self
            .conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .contains_key(&requested_conversation_id)
        {
            return Err(RuntimeError::InvalidConfig(format!(
                "conversation '{}' is already active in this runtime instance",
                requested_conversation_id
            )));
        }

        let export_event_bus: Arc<dyn EventBus> = self.event_bus.clone();
        let state_store = Arc::clone(&self.state_store);
        let coordination_backend = Arc::clone(&self.coordination_backend);
        let cluster_id = init
            .lifecycle_cluster_id
            .clone()
            .unwrap_or_else(|| self.config.runtime.cluster_id.clone());
        let runtime_instance_id = self.config.runtime.runtime_instance_id.clone();
        let catalog_lease_ttl_ms = LOCAL_LEASE_TTL_MS;
        let owner_lease_ttl_ms = LOCAL_LEASE_TTL_MS;
        let owner_lease_renew_interval =
            lease_renew_interval(owner_lease_ttl_ms, LOCAL_LEASE_RENEW_INTERVAL_MS);
        let instance_init = init.clone();

        let (info, owner_lease) = self.rt.block_on(async move {
            let conversation_id = options.conversation_id.clone();
            let Some(owner_lease) = acquire_conversation_owner_lease(
                Arc::clone(&coordination_backend),
                cluster_id.clone(),
                conversation_id.clone(),
                runtime_instance_id.clone(),
                owner_lease_ttl_ms,
                owner_lease_renew_interval,
            )
            .await?
            else {
                return Err(RuntimeError::InvalidConfig(format!(
                    "conversation '{}' is already owned by another runtime instance",
                    conversation_id
                )));
            };

            let result = async {
                let info = manager
                    .create_conversation(options, Arc::clone(&export_event_bus))
                    .await
                    .map_err(|e| RuntimeError::Internal(e.to_string()))?;

                if !skills.is_empty() {
                    let refs: Vec<&str> = skills.iter().map(String::as_str).collect();
                    manager
                        .activate_skills(&conversation_id, &refs)
                        .await
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?;
                }

                apply_conversation_init_plan(&manager, &conversation_id, &init).await?;
                for additional_agent in &init.additional_agents {
                    let refs: Vec<&str> =
                        additional_agent.skills.iter().map(String::as_str).collect();
                    manager
                        .register_agent(
                            &conversation_id,
                            additional_agent.config.clone(),
                            &refs,
                            additional_agent.max_thinking_rounds,
                            &BTreeMap::new(),
                        )
                        .await
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                }

                let lifecycle_cluster_id = init
                    .lifecycle_cluster_id
                    .clone()
                    .unwrap_or_else(|| cluster_id.clone());
                record_conversation_created(
                    Arc::clone(&state_store),
                    Arc::clone(&coordination_backend),
                    cluster_id.clone(),
                    runtime_instance_id.clone(),
                    catalog_lease_ttl_ms,
                    info.clone(),
                )
                .await?;
                publish_conversation_created_event(
                    export_event_bus,
                    &info,
                    &lifecycle_cluster_id,
                    init.lifecycle_cluster_description.as_deref().unwrap_or(""),
                )
                .await?;

                Ok::<ConversationInfo, RuntimeError>(info)
            }
            .await;

            match result {
                Ok(info) => Ok((info, owner_lease)),
                Err(error) => {
                    let _ = manager.close(&conversation_id).await;
                    let _ = owner_lease
                        .stop_and_release(Arc::clone(&coordination_backend))
                        .await;
                    Err(error)
                }
            }
        })?;

        self.conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .insert(info.conversation_id.clone(), owner_lease);
        let lifecycle_cluster_id = instance_init
            .lifecycle_cluster_id
            .clone()
            .unwrap_or_else(|| self.config.runtime.cluster_id.clone());
        let cluster_description = instance_init
            .lifecycle_cluster_description
            .clone()
            .unwrap_or_default();
        let log_path = self.create_conversation_log(
            &lifecycle_cluster_id,
            &info.conversation_id,
            info.created_at,
        );
        let log_policy = self.conversation_log_policy();
        self.conversation_instances.insert(
            info.conversation_id.clone(),
            ConversationInstanceMetadata {
                cluster_id: lifecycle_cluster_id,
                cluster_description,
                log_path,
                log_policy,
            },
        );
        self.append_conversation_log(
            &info.conversation_id,
            "conversation_created",
            json!({ "scope_id": info.scope_id }),
        );
        Ok(info)
    }

    pub(super) fn set_conversation_immutable_role_appendix(
        &self,
        conversation_id: &str,
        appendix: &str,
    ) -> Result<(), RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = conversation_id.to_string();
        let appendix = appendix.trim().to_string();
        if appendix.is_empty() {
            return Err(RuntimeError::InvalidConfig(
                "immutable role appendix must not be empty".to_string(),
            ));
        }
        self.rt.block_on(async move {
            let cache = manager
                .default_agent_cache(&conversation_id)
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            if cache
                .get::<String>(ai_assistant::context::keys::IMMUTABLE_ROLE_APPENDIX)
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))?
                .is_some()
            {
                return Err(RuntimeError::InvalidConfig(format!(
                    "conversation '{}' immutable role appendix is already set",
                    conversation_id
                )));
            }
            cache
                .set(
                    ai_assistant::context::keys::IMMUTABLE_ROLE_APPENDIX,
                    &appendix,
                    None,
                )
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))
        })
    }

    pub fn materialize_conversation(
        &mut self,
        conversation_id: &str,
        options_json: &str,
    ) -> Result<ConversationInfo, RuntimeError> {
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        let mut options = if options_json.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str::<serde_json::Value>(options_json).map_err(|e| {
                RuntimeError::InvalidConfig(format!("parse materialize options failed: {e}"))
            })?
        };
        if !options.is_object() {
            return Err(RuntimeError::InvalidConfig(
                "materialize options must be a JSON object".to_string(),
            ));
        }
        options["conversation_id"] = serde_json::Value::String(conversation_id);
        if options.get("schema").is_none() {
            options["schema"] =
                serde_json::Value::String("agent-runtime-conversation-options/v1".to_string());
        }
        self.create_conversation(&options.to_string())
    }

    #[allow(dead_code)]
    pub fn send_message(&self, conversation_id: &str, content: &str) -> Result<(), RuntimeError> {
        self.append_conversation_log(
            conversation_id,
            "send_message",
            json!({ "content_bytes": content.len() }),
        );
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        let content = content.to_string();
        let coordination_backend = Arc::clone(&self.coordination_backend);
        let lease_owner = self.config.runtime.runtime_instance_id.clone();
        let lease_ttl_ms = LOCAL_LEASE_TTL_MS;
        let lease_renew_interval =
            lease_renew_interval(lease_ttl_ms, LOCAL_LEASE_RENEW_INTERVAL_MS);
        let lease_key = format!(
            "runtime:{}:conversation:{}:turn",
            self.conversation_instances
                .get(&conversation_id)
                .map(|metadata| metadata.cluster_id.as_str())
                .unwrap_or(&self.config.runtime.cluster_id),
            conversation_id
        );
        self.rt.block_on(async move {
            let runtime_conversation_id = conversation_id.clone();
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                ttl_ms = lease_ttl_ms,
                content_len = content.len(),
                "runtime turn lease acquire start"
            );
            let acquired = coordination_backend
                .acquire_lease(&lease_key, &lease_owner, lease_ttl_ms)
                .await?;
            if !acquired {
                tracing::warn!(
                    conversation_id = %runtime_conversation_id,
                    lease_owner = %lease_owner,
                    lease_key = %lease_key,
                    "runtime turn lease acquire rejected"
                );
                return Err(RuntimeError::InvalidConfig(format!(
                    "conversation '{}' turn lease is held by another runtime",
                    runtime_conversation_id
                )));
            }
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                "runtime turn lease acquire ok"
            );

            let (stop_renewer, renewer_stop_rx) = watch::channel(false);
            let renewer = tokio::spawn(run_lease_renewer(
                Arc::clone(&coordination_backend),
                lease_key.clone(),
                lease_owner.clone(),
                lease_ttl_ms,
                lease_renew_interval,
                renewer_stop_rx,
            ));
            let send_result = manager
                .send_message(&conversation_id, &content)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()));
            let _ = stop_renewer.send(true);
            let _ = renewer.await;
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                send_ok = send_result.is_ok(),
                "runtime turn lease release start"
            );
            let release_result = coordination_backend
                .release_lease(&lease_key, &lease_owner)
                .await;
            match (send_result, release_result) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(error), Ok(())) => Err(error),
                (Ok(()), Err(error)) => Err(error),
                (Err(error), Err(release_error)) => {
                    tracing::warn!("release conversation turn lease failed: {}", release_error);
                    Err(error)
                }
            }
        })
    }

    pub fn send_message_with_admission(
        &self,
        conversation_id: &str,
        content: &str,
        command_id: String,
    ) -> Result<ai_assistant::gateway::AdmissionResult, RuntimeError> {
        self.append_conversation_log(
            conversation_id,
            "send_message_with_admission",
            json!({ "command_id": command_id.clone(), "content_bytes": content.len() }),
        );
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        let content = content.to_string();
        let coordination_backend = Arc::clone(&self.coordination_backend);
        let lease_owner = self.config.runtime.runtime_instance_id.clone();
        let lease_ttl_ms = LOCAL_LEASE_TTL_MS;
        let lease_renew_interval =
            lease_renew_interval(lease_ttl_ms, LOCAL_LEASE_RENEW_INTERVAL_MS);
        let lease_key = format!(
            "runtime:{}:conversation:{}:turn",
            self.conversation_instances
                .get(&conversation_id)
                .map(|metadata| metadata.cluster_id.as_str())
                .unwrap_or(&self.config.runtime.cluster_id),
            conversation_id
        );
        self.rt.block_on(async move {
            let runtime_conversation_id = conversation_id.clone();
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                ttl_ms = lease_ttl_ms,
                content_len = content.len(),
                "runtime turn lease acquire start"
            );
            let acquired = coordination_backend
                .acquire_lease(&lease_key, &lease_owner, lease_ttl_ms)
                .await?;
            if !acquired {
                tracing::warn!(
                    conversation_id = %runtime_conversation_id,
                    lease_owner = %lease_owner,
                    lease_key = %lease_key,
                    "runtime turn lease acquire rejected"
                );
                return Err(RuntimeError::InvalidConfig(format!(
                    "conversation '{}' turn lease is held by another runtime",
                    runtime_conversation_id
                )));
            }
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                "runtime turn lease acquire ok"
            );

            let (stop_renewer, renewer_stop_rx) = watch::channel(false);
            let renewer = tokio::spawn(run_lease_renewer(
                Arc::clone(&coordination_backend),
                lease_key.clone(),
                lease_owner.clone(),
                lease_ttl_ms,
                lease_renew_interval,
                renewer_stop_rx,
            ));
            let send_result = manager
                .send_message_with_admission(&conversation_id, &content, Some(command_id))
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()));
            let _ = stop_renewer.send(true);
            let _ = renewer.await;
            tracing::debug!(
                conversation_id = %runtime_conversation_id,
                lease_owner = %lease_owner,
                lease_key = %lease_key,
                send_ok = send_result.is_ok(),
                "runtime turn lease release start"
            );
            let release_result = coordination_backend
                .release_lease(&lease_key, &lease_owner)
                .await;
            match (send_result, release_result) {
                (Ok(admission), Ok(())) => Ok(admission),
                (Err(error), Ok(())) => Err(error),
                (Ok(_), Err(error)) => Err(error),
                (Err(error), Err(release_error)) => {
                    tracing::warn!("release conversation turn lease failed: {}", release_error);
                    Err(error)
                }
            }
        })
    }

    /// Updates one host-owned plain-text dynamic context field for one agent.
    ///
    /// Thinking reads every field in the current agent's table on entry. This
    /// operation deliberately does not expose replacement of the complete table
    /// and is not available through RPC tool HostCall.
    pub fn set_agent_dynamic_snapshot_field(
        &self,
        conversation_id: &str,
        agent_id: &str,
        field_name: &str,
        text: &str,
    ) -> Result<(), RuntimeError> {
        llm_gateway::diagnostics::append_line(format!(
            "[agent-runtime] set_agent_dynamic_snapshot_field requested conversation_id={} agent_id={} field={} bytes={}",
            conversation_id,
            agent_id,
            field_name,
            text.len()
        ));
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        let agent_id = non_empty_arg(agent_id, "agent_id")?;
        let field_name = non_empty_arg(field_name, "field_name")?;
        self.require_conversation_owner(&conversation_id)?;
        let text = text.to_string();
        self.rt.block_on(async move {
            llm_gateway::diagnostics::append_line(format!(
                "[agent-runtime] set_agent_dynamic_snapshot_field applying conversation_id={} agent_id={} field={} bytes={}",
                conversation_id,
                agent_id,
                field_name,
                text.len()
            ));
            manager
                .set_host_dynamic_snapshot_field(
                    &conversation_id,
                    &agent_id,
                    &field_name,
                    &text,
                )
                .await
                .map_err(|e| RuntimeError::InvalidConfig(e.to_string()))?;
            llm_gateway::diagnostics::append_line(format!(
                "[agent-runtime] set_agent_dynamic_snapshot_field applied conversation_id={} agent_id={} field={}",
                conversation_id,
                agent_id,
                field_name
            ));
            Ok(())
        })
    }

    pub fn resolve_tool_permission(
        &self,
        conversation_id: &str,
        tool_call_id: &str,
        decision: ai_assistant::ToolPermissionDecision,
    ) -> Result<bool, RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        let tool_call_id = non_empty_arg(tool_call_id, "tool_call_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            manager
                .resolve_tool_permission(&conversation_id, &tool_call_id, decision)
                .await
                .map_err(|error| RuntimeError::Internal(error.to_string()))
        })
    }

    fn create_conversation_log(
        &self,
        cluster_id: &str,
        conversation_id: &str,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Option<PathBuf> {
        let resources = self.registries.resources.as_ref();
        let logs_dir = resources
            .and_then(|resources| resources.logs_dir.clone())
            .or_else(|| {
                self.config
                    .runtime
                    .data_dir
                    .as_ref()
                    .map(|data_dir| data_dir.join("logs"))
            });
        let policy = resources
            .map(|resources| resources.conversation_log_policy.clone())
            .unwrap_or_default();
        create_conversation_log_path(
            logs_dir.as_deref(),
            cluster_id,
            conversation_id,
            created_at,
            &policy,
        )
    }

    fn conversation_log_policy(&self) -> ConversationLogPolicy {
        self.registries
            .resources
            .as_ref()
            .map(|resources| resources.conversation_log_policy.clone())
            .unwrap_or_default()
    }

    fn append_conversation_log(&self, conversation_id: &str, event: &str, details: Value) {
        let Some(metadata) = self.conversation_instances.get(conversation_id) else {
            return;
        };
        let Some(path) = metadata.log_path.as_ref() else {
            return;
        };
        append_conversation_log_path(
            Some(path),
            &self.config.runtime.runtime_instance_id,
            &metadata.cluster_id,
            conversation_id,
            event,
            details,
            metadata.log_policy.max_file_bytes,
        );
    }

    pub fn set_conversation_summary_model_with_admission(
        &self,
        conversation_id: &str,
        model_name: &str,
        command_id: String,
    ) -> Result<ai_assistant::gateway::AdmissionResult, RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        let model_name = non_empty_arg(model_name, "model_name")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            manager
                .set_summary_model_with_admission(&conversation_id, &model_name, Some(command_id))
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))
        })
    }

    /// Set the summarization model used by `compact_history`.
    ///
    /// This only updates the conversation-level summary model; it does not
    /// change the active reasoning model for future assistant turns.
    #[allow(dead_code)]
    pub fn set_conversation_summary_model(
        &self,
        conversation_id: &str,
        model_name: &str,
    ) -> Result<(), RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        let model_name = non_empty_arg(model_name, "model_name")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            manager
                .set_summary_model(&conversation_id, &model_name)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))
        })
    }

    pub fn compact_conversation_history_with_admission(
        &self,
        conversation_id: &str,
        agent_ids: Vec<String>,
        command_id: String,
    ) -> Result<(ai_assistant::gateway::AdmissionResult, String), RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            let (admission, report) = manager
                .compact_history_with_admission(&conversation_id, agent_ids, Some(command_id))
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            let report_json = serde_json::to_string(&report).map_err(|e| {
                RuntimeError::Internal(format!("serialize compact history report failed: {e}"))
            })?;
            Ok((admission, report_json))
        })
    }

    /// Compact conversation history and return the serialized per-agent report.
    ///
    /// An empty `agent_ids` list compacts the whole cluster; otherwise only the
    /// named agents are compacted.
    #[allow(dead_code)]
    pub fn compact_conversation_history(
        &self,
        conversation_id: &str,
        agent_ids: Vec<String>,
    ) -> Result<String, RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            let report = manager
                .compact_history(&conversation_id, agent_ids)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            serde_json::to_string(&report).map_err(|e| {
                RuntimeError::Internal(format!("serialize compact history report failed: {e}"))
            })
        })
    }

    #[allow(dead_code)]
    pub fn pause_conversation(&self, conversation_id: &str) -> Result<(), RuntimeError> {
        self.append_conversation_log(
            conversation_id,
            "pause_conversation",
            Value::Object(Default::default()),
        );
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            manager
                .request_pause(&conversation_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))
        })
    }

    pub fn pause_conversation_with_admission(
        &self,
        conversation_id: &str,
        command_id: String,
    ) -> Result<ai_assistant::gateway::AdmissionResult, RuntimeError> {
        self.append_conversation_log(
            conversation_id,
            "pause_conversation_with_admission",
            json!({ "command_id": command_id.clone() }),
        );
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.require_conversation_owner(&conversation_id)?;
        self.rt.block_on(async move {
            manager
                .request_pause_with_admission(&conversation_id, Some(command_id))
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))
        })
    }

    pub fn close_conversation(&mut self, conversation_id: &str) -> Result<(), RuntimeError> {
        let manager = self.manager()?;
        let conversation_id = non_empty_arg(conversation_id, "conversation_id")?;
        self.append_conversation_log(
            &conversation_id,
            "conversation_closing",
            Value::Object(Default::default()),
        );
        let owner_lease = self
            .conversation_owner_leases
            .lock()
            .map_err(|_| RuntimeError::Internal("owner lease lock poisoned".to_string()))?
            .remove(&conversation_id)
            .ok_or_else(|| {
                RuntimeError::InvalidConfig(format!(
                    "conversation '{}' is not owned by this runtime instance",
                    conversation_id
                ))
            })?;
        let state_store = Arc::clone(&self.state_store);
        let coordination_backend = Arc::clone(&self.coordination_backend);
        let metadata = self
            .conversation_instances
            .get(&conversation_id)
            .cloned()
            .unwrap_or_else(|| ConversationInstanceMetadata {
                cluster_id: self.config.runtime.cluster_id.clone(),
                cluster_description: String::new(),
                log_path: None,
                log_policy: self.conversation_log_policy(),
            });
        let cluster_id = metadata.cluster_id.clone();
        let cluster_description = metadata.cluster_description.clone();
        let export_event_bus: Arc<dyn EventBus> = self.event_bus.clone();
        let runtime_instance_id = self.config.runtime.runtime_instance_id.clone();
        let catalog_lease_ttl_ms = LOCAL_LEASE_TTL_MS;
        let closing_conversation_id = conversation_id.clone();
        let close_result = self.rt.block_on(async move {
            let close_result = async {
                let removed = manager
                    .close(&closing_conversation_id)
                    .await
                    .map_err(|e| RuntimeError::Internal(e.to_string()))?;
                if removed {
                    record_conversation_closed(
                        state_store,
                        Arc::clone(&coordination_backend),
                        cluster_id.clone(),
                        runtime_instance_id,
                        catalog_lease_ttl_ms,
                        closing_conversation_id.clone(),
                    )
                    .await?;
                    publish_conversation_closed_event(
                        export_event_bus,
                        &closing_conversation_id,
                        &cluster_id,
                        &cluster_description,
                    )
                    .await?;
                }
                Ok::<(), RuntimeError>(())
            }
            .await;
            let release_result = owner_lease
                .stop_and_release(Arc::clone(&coordination_backend))
                .await;
            match (close_result, release_result) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(error), Ok(())) => Err(error),
                (Ok(()), Err(error)) => Err(error),
                (Err(error), Err(release_error)) => {
                    tracing::warn!("release conversation owner lease failed: {}", release_error);
                    Err(error)
                }
            }
        });
        self.append_conversation_log(
            &conversation_id,
            "conversation_closed",
            json!({ "success": close_result.is_ok() }),
        );
        self.conversation_instances.remove(&conversation_id);
        close_result
    }
}
