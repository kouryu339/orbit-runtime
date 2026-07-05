use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetToolContract {
    pub name: String,
    pub description: String,
    pub parameters_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetWhiteboxContract {
    pub target_agent_id: String,
    pub role_skill: String,
    pub feature_skills: Vec<String>,
    pub tools: Vec<TargetToolContract>,
    pub developer_brief: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversaryPersona {
    pub identity: String,
    pub personality: String,
    pub background: String,
    pub goal: String,
    pub strategy: String,
    #[serde(default)]
    pub hidden_facts: Vec<String>,
    #[serde(default)]
    pub boundaries: Vec<String>,
}
