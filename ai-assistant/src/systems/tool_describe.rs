//! Full, on-demand tool descriptors for Agent and Workflow authoring.

use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput};
use corework::define_operation;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;
use std::collections::HashSet;

#[define_operation(
    name = "ToolDescribe",
    display_name = "读取工具完整说明{tool_names}，返回{tools}",
    category = "Assistant",
    system_only,
    description = "按名称读取当前已激活工具的完整输入、输出、行为与 Workflow 引脚说明。通用工具在具体使用、编码或编写 Workflow 前建议先读取；读取说明不会改变工具权限或激活状态。",
    params {
        tool_names: "Array<String>@必填。要读取的工具名称数组，最多 32 个；名称必须来自当前 Available Tools。"
    },
    outputs {
        tools: "Array<Any>@按请求顺序返回的完整工具 Descriptor，包含 schema_revision 和 descriptor_hash。"
    },
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct ToolDescribeSystem;

#[async_trait]
impl SystemOperation for ToolDescribeSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(args) => args,
            Err(output) => return Ok(output),
        };
        let raw = match args.safe_require("tool_names") {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };
        let names = match parse_tool_names(&raw) {
            Ok(names) => names,
            Err(message) => return Ok(AIOutput::error(400, message)),
        };
        let active = crate::AssistantContext::all_active_tools(&ctx.cache).await?;
        let active = active.into_iter().collect::<HashSet<_>>();
        let unavailable = names
            .iter()
            .filter(|name| !active.contains(*name))
            .cloned()
            .collect::<Vec<_>>();
        if !unavailable.is_empty() {
            return Ok(AIOutput::error(
                404,
                format!(
                    "ToolDescribe can only read current Available Tools; unavailable: {}",
                    unavailable.join(", ")
                ),
            ));
        }
        let descriptors = match crate::tool_schema::describe_registered_tools(&names) {
            Ok(descriptors) => descriptors,
            Err(message) => return Ok(AIOutput::error(404, message)),
        };
        let result = serde_json::json!({"tools": descriptors});
        let text = serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
        Ok(AIOutput::success(result, text))
    }

    fn name(&self) -> &str {
        "ToolDescribe"
    }
}

fn parse_tool_names(raw: &str) -> Result<Vec<String>, String> {
    let mut names = if raw.trim_start().starts_with('[') {
        serde_json::from_str::<Vec<String>>(raw)
            .map_err(|error| format!("tool_names must be a JSON string array: {error}"))?
    } else {
        raw.split(',').map(str::to_string).collect()
    };
    for name in &mut names {
        *name = name.trim().to_string();
    }
    names.retain(|name| !name.is_empty());
    let mut seen = HashSet::new();
    names.retain(|name| seen.insert(name.clone()));
    if names.is_empty() {
        return Err("tool_names must contain at least one tool name".to_string());
    }
    if names.len() > 32 {
        return Err("tool_names may contain at most 32 tool names".to_string());
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corework::execution_unit::{ExecutionUnit, UnitType};
    use corework::world::FrameworkState;
    use std::sync::Arc;

    #[test]
    fn parses_json_and_deduplicates_names() {
        assert_eq!(
            parse_tool_names(r#"["Grep", "Grep", "HistoryRead"]"#).unwrap(),
            vec!["Grep", "HistoryRead"]
        );
    }

    #[tokio::test]
    async fn returns_full_descriptor_for_an_active_tool() {
        let framework = FrameworkState::initialize().unwrap();
        let unit = Arc::new(ExecutionUnit::new_root(UnitType::StateMachine, framework));
        let ctx = unit.create_context();
        crate::AssistantContext::set_active_tools(
            &ctx.cache,
            vec!["ToolDescribe".to_string(), "Wait".to_string()],
        )
        .await
        .unwrap();

        let output = ToolDescribeSystem
            .execute(
                AIInput {
                    input: r#"--tool_names ["Wait"]"#.to_string(),
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(output.error_code, 0);
        assert_eq!(output.result["tools"][0]["name"], "Wait");
        assert!(output.result["tools"][0]["outputs"]
            .as_array()
            .is_some_and(|outputs| !outputs.is_empty()));
        assert!(output.result["tools"][0]["descriptor_hash"]
            .as_str()
            .is_some());
    }
}
