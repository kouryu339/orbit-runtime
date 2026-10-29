//! Content-only cluster admission. Browser v1 deliberately excludes file profiles.
use ai_assistant::{AIAssistantConfig, SkillManager, ToolPermissionPolicy};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cluster {
    pub schema: String,
    pub id: String,
    pub description: String,
    #[serde(alias = "focusAgentId")]
    pub focus_agent_id: String,
    pub agents: Vec<Agent>,
    #[serde(alias = "maxThinkingRounds")]
    pub max_thinking_rounds: u32,
    pub permissions: ToolPermissionPolicy,
}
impl Default for Cluster {
    fn default() -> Self {
        Self {
            schema: "agent-runtime-agent-cluster-registration/v1".into(),
            id: String::new(),
            description: String::new(),
            focus_agent_id: String::new(),
            agents: vec![],
            max_thinking_rounds: 0,
            permissions: Default::default(),
        }
    }
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub role: String,
    pub features: Vec<String>,
    #[serde(alias = "modelUid")]
    pub model_uid: Option<u32>,
    #[serde(alias = "systemSkills")]
    pub system_skills: BTreeMap<String, String>,
    #[serde(alias = "toolProtocol")]
    pub tool_protocol: ai_assistant::ToolProtocol,
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
}
impl Cluster {
    pub fn validate(&mut self, skills: &SkillManager) -> Result<(), String> {
        if self.schema != "agent-runtime-agent-cluster-registration/v1" || !valid_id(&self.id) {
            return Err("invalid cluster schema or id".into());
        }
        if self.agents.is_empty() || self.agents.len() > 64 {
            return Err("cluster requires 1..64 Agents".into());
        }
        if self.focus_agent_id.is_empty() {
            self.focus_agent_id = self.agents[0].id.clone();
        }
        let mut ids = BTreeSet::new();
        for agent in &self.agents {
            if !valid_id(&agent.id) || !ids.insert(&agent.id) {
                return Err("invalid or duplicate Agent id".into());
            }
            let role = skills
                .get(&agent.role)
                .ok_or_else(|| format!("Role Skill '{}' is not registered", agent.role))?;
            if !role.metadata.is_role() {
                return Err(format!("Skill '{}' must be a Role", agent.role));
            }
            let mut names = BTreeSet::new();
            for feature in &agent.features {
                let skill = skills
                    .get(feature)
                    .ok_or_else(|| format!("Feature Skill '{feature}' is not registered"))?;
                if skill.metadata.is_role() || skill.metadata.system_layer || !names.insert(feature)
                {
                    return Err(format!("invalid or duplicate Feature Skill '{feature}'"));
                }
            }
            for skill in agent.system_skills.values() {
                if !skills
                    .get(skill)
                    .is_some_and(|skill| skill.metadata.system_layer)
                {
                    return Err(format!("System Skill '{skill}' is not registered"));
                }
            }
            if let Some(uid) = agent.model_uid {
                if llm_gateway::key_store::get(uid).is_none() {
                    return Err(format!("unknown model_uid: {uid}"));
                }
            }
        }
        if !ids.contains(&self.focus_agent_id) {
            return Err("focus Agent does not exist".into());
        }
        Ok(())
    }
}
impl Agent {
    pub fn config(&self, language: &str) -> AIAssistantConfig {
        let mut config = AIAssistantConfig::default();
        config.agent_id = self.id.clone();
        config.name = if self.name.is_empty() {
            self.id.clone()
        } else {
            self.name.clone()
        };
        config.language = language.into();
        config.model = self
            .model_uid
            .map(|uid| uid.to_string())
            .unwrap_or_else(|| "default".into());
        config.system_skills = self.system_skills.clone();
        config
            .system_skills
            .entry("thinking".into())
            .or_insert_with(|| "thinking-pro".into());
        config.tool_protocol = self.tool_protocol;
        config
    }
    pub fn skill_names(&self) -> Vec<&str> {
        std::iter::once(self.role.as_str())
            .chain(self.features.iter().map(String::as_str))
            .collect()
    }
}
