use crate::error::{FrameworkError, Result};
use crate::jev::JevDefinition;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JevDecision {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

#[async_trait]
pub trait JevClient: Send + Sync {
    async fn choose(&self, definition: &JevDefinition, state: &Value) -> Result<JevDecision>;
}

#[derive(Clone)]
pub struct HttpJevClient {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
}

impl HttpJevClient {
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        Self::with_endpoint(api_key, "https://api.typesafe.ai/v1/systemone")
    }

    pub fn with_endpoint(api_key: impl Into<String>, endpoint: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(FrameworkError::ValidationError(
                "Jev API key must not be empty".to_string(),
            ));
        }
        let endpoint = endpoint.into();
        if endpoint.trim().is_empty() {
            return Err(FrameworkError::ValidationError(
                "Jev endpoint must not be empty".to_string(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| {
                FrameworkError::SystemError(format!("build Jev HTTP client failed: {error}"))
            })?;
        Ok(Self {
            client,
            endpoint,
            api_key,
        })
    }
}

#[async_trait]
impl JevClient for HttpJevClient {
    async fn choose(&self, definition: &JevDefinition, state: &Value) -> Result<JevDecision> {
        let criteria = definition
            .actions
            .iter()
            .map(|(name, action)| (name.clone(), Value::String(action.description.clone())))
            .collect::<BTreeMap<_, _>>();
        let payload = json!({
            "model": definition.model,
            "state": state,
            "questions": {"next_action": {
                "type": "choice",
                "instructions": definition.instructions,
                "criteria": criteria
            }}
        });
        let mut attempt = 0u32;
        let (status, body) = loop {
            attempt += 1;
            let response = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.api_key)
                .json(&payload)
                .send()
                .await;
            match response {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.map_err(|error| {
                        FrameworkError::InvalidData(format!(
                            "read Jev response body failed: {error}"
                        ))
                    })?;
                    let body = serde_json::from_str::<Value>(&text).map_err(|error| {
                        FrameworkError::InvalidData(format!(
                            "decode Jev response failed: {error}; body={text}"
                        ))
                    })?;
                    if (status.as_u16() == 429 || status.is_server_error()) && attempt < 3 {
                        tokio::time::sleep(Duration::from_millis(200 * (1 << (attempt - 1)))).await;
                        continue;
                    }
                    break (status, body);
                }
                Err(error) if attempt < 3 && (error.is_timeout() || error.is_connect()) => {
                    tokio::time::sleep(Duration::from_millis(200 * (1 << (attempt - 1)))).await;
                }
                Err(error) => {
                    return Err(FrameworkError::SystemError(format!(
                        "Jev request failed after {attempt} attempt(s): {error}"
                    )));
                }
            }
        };
        if !status.is_success() {
            return Err(FrameworkError::SystemError(format!(
                "Jev API returned HTTP {status}: {body}"
            )));
        }
        let answer = body.pointer("/answers/next_action").ok_or_else(|| {
            FrameworkError::InvalidData("Jev response is missing answers.next_action".to_string())
        })?;
        let action = answer
            .get("choice")
            .and_then(Value::as_str)
            .or_else(|| answer.as_str())
            .ok_or_else(|| {
                FrameworkError::InvalidData(
                    "Jev response next_action is missing choice".to_string(),
                )
            })?
            .to_string();
        let confidence = answer.get("confidence").and_then(Value::as_f64);
        Ok(JevDecision { action, confidence })
    }
}
