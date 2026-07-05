//! LLM 对话节点

use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput};
use corework::define_operation;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;

use crate::dispatch::call_llm;
use crate::types::ChatMessage;

// ==================== CallLlm ====================

#[define_operation(
    name = "CallLlm",
    description = "调用 LLM 对话：{{system_message}} 设定角色，{{user_message}} 为问题，返回 {{response_text}}。\
                   {{model_id}} 填写在「LLM 配置」中启用的模型 ID（整数），\
                   可在应用设置页查看每个已启用模型对应的 ID。\
                   {{temperature}} 采样温度 0.0-2.0，留空使用默认值。",
    category = "AI/LLM",
    params {
        user_message:   "String@用户消息内容（必填）",
        system_message: "String@系统提示词，设定 AI 角色（可选）",
        model_id:       "i64@已启用模型的注册 ID，在设置→LLM 配置中查看",
        temperature:    "f64@采样温度 0.0-2.0（可选，留空使用模型默认值）",
        max_tokens:     "i64@最大输出 Token 数（可选）",
    },
    outputs {
        response_text:  "String@模型回复的文本内容",
        input_tokens:   "i64@消耗的输入 Token 数",
        output_tokens:  "i64@消耗的输出 Token 数",
    },
    exec_in  = ["In@启动对话"],
    exec_out = ["Then@对话完成"],
    destructive = false,
    readonly    = true,
    idempotent  = false,
    open_world  = true,
)]
pub struct CallLlm;

#[async_trait]
impl SystemOperation for CallLlm {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: AIInput, _ctx: &Context) -> Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };
        let user_message = match args.safe_require("user_message") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };

        // model_id 是 u32 整数，对应用户在设置中启用的模型
        let model_id: u32 = match args.get("model_id").and_then(|s| s.parse::<u32>().ok()) {
            Some(id) => id,
            None => {
                return Ok(AIOutput::error(
                    400,
                    "model_id 必须为整数，请在「设置 → LLM 配置」中查看已启用模型的 ID".to_string(),
                ))
            }
        };

        let system_message = args.get("system_message").map(|s| s.to_string());
        let temperature = args.get("temperature").and_then(|s| s.parse::<f64>().ok());
        let max_tokens = args.get("max_tokens").and_then(|s| s.parse::<u32>().ok());

        let mut messages: Vec<ChatMessage> = Vec::new();
        if let Some(sys) = system_message {
            if !sys.is_empty() {
                messages.push(ChatMessage::system(sys));
            }
        }
        messages.push(ChatMessage::user(user_message));

        match call_llm(model_id, &messages, temperature, None, max_tokens).await {
            Ok(resp) => {
                let input_tokens = resp.tokens.as_ref().map(|t| t.input_tokens).unwrap_or(0);
                let output_tokens = resp.tokens.as_ref().map(|t| t.output_tokens).unwrap_or(0);
                Ok(AIOutput::success(
                    serde_json::json!({
                        "response_text": resp.content,
                        "input_tokens": input_tokens,
                        "output_tokens": output_tokens,
                    }),
                    format!(
                        "[成功] LLM 回复（{}字，消耗 {}+{} tokens）",
                        resp.content.len(),
                        input_tokens,
                        output_tokens
                    ),
                ))
            }
            Err(e) => Ok(AIOutput::error(500, format!("LLM 调用失败: {}", e))),
        }
    }

    fn name(&self) -> &str {
        "CallLlm"
    }
}
