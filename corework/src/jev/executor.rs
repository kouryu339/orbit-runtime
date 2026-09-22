use crate::error::{FrameworkError, Result};
use crate::jev::{JevExecutionContext, JevSnapshotUpdate, JevToolExecutor, JevToolResult};
use crate::orchestration::Context;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// Executes a selected action through the existing dynamic system registry.
///
/// This executor is intended for host-owned Jev runs. AI-owned runs should
/// wrap it with the product's existing permission policy before delegation.
#[derive(Debug, Default)]
pub struct RegistryJevToolExecutor;

#[async_trait]
impl JevToolExecutor for RegistryJevToolExecutor {
    async fn execute(
        &self,
        tool: &str,
        arguments: BTreeMap<String, Value>,
        context: &Context,
        _jev: &JevExecutionContext,
    ) -> Result<JevToolResult> {
        let jev_enabled = context
            .get_registry()
            .get_dynamic_metadata(tool)
            .map(|metadata| metadata.jev_enabled)
            .or_else(|| {
                inventory::iter::<crate::ai_system::AISystemFactory>()
                    .find(|factory| factory.metadata.name == tool)
                    .map(|factory| factory.metadata.jev_enabled)
            })
            .unwrap_or(false);
        if !jev_enabled {
            return Err(FrameworkError::InvalidOperation(format!(
                "Tool '{tool}' is not enabled for Jev execution"
            )));
        }
        let executor = context.get_dynamic_system(tool)?;
        let output = executor
            .execute_dynamic(arguments.into_iter().collect::<HashMap<_, _>>(), context)
            .await?;
        let object = output.as_object().ok_or_else(|| {
            FrameworkError::InvalidData(format!("Jev tool '{tool}' returned a non-object envelope"))
        })?;
        let mut result = object.get("result").cloned().unwrap_or(Value::Null);
        let to_ai = object
            .get("to_ai")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let error_code = object
            .get("error_code")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or_default();
        let snapshot_value = result
            .as_object_mut()
            .and_then(|value| value.remove("jev_snapshot_update"));
        let snapshot_update = match snapshot_value {
            Some(value) => serde_json::from_value::<JevSnapshotUpdate>(value).map_err(|error| {
                FrameworkError::InvalidData(format!(
                    "Jev tool '{tool}' returned invalid jev_snapshot_update: {error}"
                ))
            })?,
            None => JevSnapshotUpdate::default(),
        };
        Ok(JevToolResult {
            result,
            to_ai,
            error_code,
            snapshot_update,
        })
    }
}
