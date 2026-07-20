use super::*;

impl RuntimeFacade {
    pub fn configure_providers(&mut self, providers_input: &str) -> Result<(), RuntimeError> {
        let content = read_json_or_file(providers_input)?;

        if let Ok(config_v1) = serde_json::from_str::<ProviderConfigV1>(&content) {
            if config_v1.schema == "agent-runtime-provider-config/v1" {
                let current_model_uid = config_v1.current_model_uid;
                let config = llm_gateway::LlmConfig {
                    providers: config_v1.providers.into_iter().map(Into::into).collect(),
                    current_model_uid,
                };
                validate_llm_config(&config)?;
                validate_current_model(current_model_uid, &config)?;
                llm_gateway::build_index_and_resolver(config.clone());
                if let Some(uid) = current_model_uid {
                    key_store::set_current(uid);
                }
                self.llm_config = config;
                self.sync_provider_bundle();
                if let Some(bundle) = self.provider_bundle.as_mut() {
                    bundle.current_model_uid = current_model_uid;
                }
                self.persist_provider_config_text(&content)?;
                return Ok(());
            }
        }

        if let Ok(config) = serde_json::from_str::<llm_gateway::LlmConfig>(&content) {
            validate_llm_config(&config)?;
            llm_gateway::build_index_and_resolver(config.clone());
            self.llm_config = config;
            self.sync_provider_bundle();
            self.persist_llm_config()?;
            return Ok(());
        }

        let bundle: ProviderBundle = serde_json::from_str(&content).map_err(|e| {
            RuntimeError::InvalidConfig(format!("parse providers_json failed: {e}"))
        })?;

        let provider_map: std::collections::HashMap<u32, ProviderConfig> = bundle
            .providers
            .clone()
            .into_iter()
            .map(|provider| (provider.uid, provider))
            .collect();
        for provider in provider_map.values() {
            if provider.prompt_cache_control
                && provider.api_paradigm != Some(llm_gateway::ApiParadigm::AnthropicMessages)
            {
                return Err(RuntimeError::InvalidConfig(format!(
                    "providers['{}'].prompt_cache_control requires api_paradigm 'anthropic_messages'",
                    provider.uid
                )));
            }
        }

        let entries = bundle.models.clone().into_iter().map(|model| {
            (
                model.uid,
                key_store::ModelEntry {
                    model_name: model.model_name,
                    provider_uid: model.provider_uid,
                    context_window: model.context_window,
                },
            )
        });
        key_store::reload(entries);

        key_store::set_resolver(move |provider_uid| {
            provider_map
                .get(&provider_uid)
                .map(|provider| key_store::ProviderRuntimeConfig {
                    api_key: provider.api_key.clone(),
                    base_url: provider.base_url.clone(),
                    api_paradigm: provider.api_paradigm,
                    prompt_cache_control: provider.prompt_cache_control,
                })
        });

        if let Some(uid) = bundle.current_model_uid {
            if !bundle.models.iter().any(|model| model.uid == uid) {
                return Err(RuntimeError::Llm(format!(
                    "current_model_uid {uid} is not configured"
                )));
            }
            key_store::set_current(uid);
        }

        self.provider_bundle = Some(bundle);
        self.persist_provider_config_text(&content)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn add_providers_typed(&mut self, providers_json: &str) -> Result<(), RuntimeError> {
        let providers = parse_provider_array(providers_json)?;

        for provider in providers {
            if let Some(existing) = self
                .llm_config
                .providers
                .iter_mut()
                .find(|p| p.id == provider.id)
            {
                for model in provider.enabled_models {
                    if !existing.enabled_models.iter().any(|m| m.uid == model.uid) {
                        existing.enabled_models.push(model);
                    }
                }
                // Keep provider display metadata in sync with imported host config.
                existing.name = provider.name;
                existing.api_key = provider.api_key;
                if !provider.base_url.is_empty() {
                    existing.base_url = provider.base_url;
                }
                existing.api_paradigm = provider.api_paradigm;
                existing.prompt_cache_control = provider.prompt_cache_control;
            } else {
                self.llm_config.providers.push(provider);
            }
        }

        validate_llm_config(&self.llm_config)?;
        llm_gateway::build_index_and_resolver(self.llm_config.clone());
        self.sync_provider_bundle();
        self.persist_llm_config()?;

        Ok(())
    }

    pub(super) fn sync_provider_bundle(&mut self) {
        let bundle = self.provider_bundle.get_or_insert_with(Default::default);
        bundle.providers = self
            .llm_config
            .providers
            .iter()
            .map(|p| ProviderConfig {
                uid: p.id,
                name: Some(p.name.clone()),
                api_key: p.api_key.clone(),
                base_url: p.base_url.clone(),
                api_paradigm: p.api_paradigm,
                prompt_cache_control: p.prompt_cache_control,
            })
            .collect();
        bundle.models = self
            .llm_config
            .providers
            .iter()
            .flat_map(|p| {
                p.enabled_models.iter().map(move |m| ModelConfig {
                    uid: m.uid,
                    provider_uid: p.id,
                    model_name: m.model_id.clone(),
                    context_window: 0,
                })
            })
            .collect();
        bundle.current_model_uid = self
            .llm_config
            .current_model_uid
            .or_else(key_store::current);
    }

    pub fn provider_definitions(&self) -> Result<String, RuntimeError> {
        let Some(bundle) = &self.provider_bundle else {
            return Ok(json!({
                "schema": "agent-runtime-provider-definitions/v1",
                "providers": [],
                "models": [],
                "current_model_uid": null
            })
            .to_string());
        };

        let definitions = ProviderDefinitions {
            schema: "agent-runtime-provider-definitions/v1",
            providers: bundle
                .providers
                .iter()
                .map(|provider| ProviderDefinition {
                    uid: provider.uid,
                    name: provider.name.clone(),
                    base_url: provider.base_url.clone(),
                    api_paradigm: provider.api_paradigm,
                    prompt_cache_control: provider.prompt_cache_control,
                    api_key_set: !provider.api_key.is_empty(),
                })
                .collect(),
            models: bundle.models.clone(),
            current_model_uid: bundle.current_model_uid,
        };

        serde_json::to_string(&definitions).map_err(|e| {
            RuntimeError::Internal(format!("serialize provider definitions failed: {e}"))
        })
    }

    pub fn set_current_model(&mut self, model_uid: u32) -> Result<(), RuntimeError> {
        if key_store::get(model_uid).is_none() {
            return Err(RuntimeError::Llm(format!(
                "model uid {model_uid} is not configured"
            )));
        }
        key_store::set_current(model_uid);
        self.llm_config.current_model_uid = Some(model_uid);
        if let Some(bundle) = self.provider_bundle.as_mut() {
            bundle.current_model_uid = Some(model_uid);
        }
        self.persist_current_model_selection(model_uid)?;
        Ok(())
    }

    pub fn set_language(&mut self, language: &str) -> Result<(), RuntimeError> {
        let normalized = ai_assistant::prompt_assets::set_language(language)
            .map_err(RuntimeError::InvalidConfig)?;
        self.config.runtime.language = normalized.clone();
        if let Some(manager) = self.conversation_manager.clone() {
            self.rt.block_on(async move {
                for info in manager.list().await {
                    manager
                        .set_language(&info.conversation_id, &normalized)
                        .await
                        .map_err(|e| RuntimeError::Internal(e.to_string()))?;
                }
                Ok::<(), RuntimeError>(())
            })?;
        }
        Ok(())
    }

    pub(super) fn persist_provider_config_text(&self, content: &str) -> Result<(), RuntimeError> {
        let Some(path) = self.config.runtime.llm_config_path.clone() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                RuntimeError::InvalidConfig(format!(
                    "create provider config dir failed {}: {e}",
                    parent.display()
                ))
            })?;
        }
        fs::write(&path, content).map_err(|e| {
            RuntimeError::InvalidConfig(format!(
                "write provider config failed {}: {e}",
                path.display()
            ))
        })
    }

    pub(super) fn persist_llm_config(&self) -> Result<(), RuntimeError> {
        let content = serde_json::to_string_pretty(&self.llm_config).map_err(|e| {
            RuntimeError::Internal(format!("serialize provider config failed: {e}"))
        })?;
        self.persist_provider_config_text(&content)
    }

    fn persist_current_model_selection(&self, model_uid: u32) -> Result<(), RuntimeError> {
        let Some(path) = self.config.runtime.llm_config_path.clone() else {
            return Ok(());
        };
        if !path.exists() {
            return self.persist_llm_config();
        }

        let content = fs::read_to_string(&path).map_err(|e| {
            RuntimeError::InvalidConfig(format!(
                "read provider config failed {}: {e}",
                path.display()
            ))
        })?;
        let mut value: Value = serde_json::from_str(&content).map_err(|e| {
            RuntimeError::InvalidConfig(format!(
                "parse provider config failed {}: {e}",
                path.display()
            ))
        })?;

        let Some(object) = value.as_object_mut() else {
            return self.persist_llm_config();
        };
        let field = if object.contains_key("currentModelUid")
            && !object.contains_key("current_model_uid")
        {
            "currentModelUid"
        } else {
            "current_model_uid"
        };
        object.insert(field.to_string(), json!(model_uid));

        let content = serde_json::to_string_pretty(&value).map_err(|e| {
            RuntimeError::Internal(format!("serialize provider config failed: {e}"))
        })?;
        self.persist_provider_config_text(&content)
    }
}
