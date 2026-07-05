//!
//! - `WfListWorkflows`：列出已注册工作流
//! - `WfRunWorkflow`：按名称执行已注册工作流
//! - `WfRunScript`：直接执行操作链文本（不保存）
//! - `WfReviseWorkflow`：修改已有工作流

use async_trait::async_trait;
use corework::ai_system::{AIInput, AIOutput};
use corework::buns_system;
use corework::cache::CacheExt;
use corework::error::FrameworkError;
use corework::orchestration::Context;
use corework::system::SystemOperation;
use corework::workflow::blueprint_json::{
    BlueprintJson, BlueprintNodeJson, BlueprintVisibility, ConnectionJson,
};
use corework::workflow::core::DataValue;
use corework::workflow::BlueprintLoader;
use llm_gateway;

// ============================================================================
// ============================================================================

pub mod build_system_names {
    pub const BUILD_WORKFLOW_FROM_CHAIN: &str = "BuildWorkflowFromChainSystem";
    pub const LIST_WORKFLOWS: &str = "WfListWorkflows";
    pub const RUN_WORKFLOW: &str = "WfRunWorkflow";
    pub const RUN_SCRIPT: &str = "WfRunScript";
    pub const REVISE_WORKFLOW: &str = "WfReviseWorkflow";
}

// ============================================================================
// BuildWorkflowFromChainSystem
// ============================================================================

#[buns_system(
    "BuildWorkflowFromChainSystem",
    description = "将{{chain}}操作链编译为{{name}}工作流（含{{description}}描述），驱动LLM生成节点并验证",
    params {
        chain:        "结构化操作链文本（必填）：带步骤编号的伪代码，支持 IF/FOR/BREAK/$变量 语法",
        action_nodes: "本次工作流用到的动作节点类型名（必填，逗号分隔），如 FillInput,ClickElement",
        name:         "工作流名称（必填）",
        description:  "工作流描述（可选）",
        inputs:       "工作流入参描述（可选），JSON 数组格式",
        outputs:      "工作流出参描述（可选），JSON 数组格式"
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct BuildWorkflowFromChainSystem;

#[async_trait]
impl SystemOperation for BuildWorkflowFromChainSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };

        let chain = match args.safe_require("chain") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let action_nodes_raw = match args.safe_require("action_nodes") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let workflow_name = match args.safe_require("name") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let workflow_desc = args.get_or("description", "");
        let inputs_desc = args.get_or("inputs", "");
        let outputs_desc = args.get_or("outputs", "");

        let action_node_types: Vec<&str> = action_nodes_raw
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        tracing::debug!(
            "BuildWorkflowFromChain: name={}, action_nodes={:?}",
            workflow_name,
            action_node_types
        );

        let system_prompt = crate::workflow::workflows::draft::build_chain_compiler_prompt(
            &action_node_types,
            &workflow_name,
            &inputs_desc,
            &outputs_desc,
        );

        let model: Option<String> = ctx.cache.get("model").await.unwrap_or(None);
        let model_name = model
            .as_deref()
            .filter(|m| *m != "default")
            .unwrap_or("qwen-plus");
        let model_uid = llm_gateway::key_store::find_by_name(model_name).ok_or_else(|| {
            FrameworkError::SystemError(format!(
                "未找到模型 '{}' 的 uid，请先在设置中配置",
                model_name
            ))
        })?;

        let mut last_error = String::new();
        let mut error_context = String::new();

        #[derive(serde::Deserialize)]
        struct PartialBp {
            #[serde(default)]
            nodes: Vec<BlueprintNodeJson>,
            #[serde(default)]
            connections: Vec<ConnectionJson>,
        }

        for attempt in 1..=3u32 {
            let user_content = if error_context.is_empty() {
                format!("操作链：\n{}", chain)
            } else {
                format!(
                    "操作链：\n{}\n\n上次错误，请修正：\n{}",
                    chain, error_context
                )
            };

            let messages = vec![
                llm_gateway::ChatMessage::system_cached(system_prompt.clone()),
                llm_gateway::ChatMessage {
                    role: "user".to_string(),
                    content: user_content,
                    cache_control: false,
                    tool_call_id: None,
                    name: None,
                    tool_calls: None,
                    reasoning_content: None,
                },
            ];

            tracing::debug!("BuildWorkflowFromChain 第 {} 次尝试", attempt);

            let response = llm_gateway::call_llm(model_uid, &messages, None, None, None)
                .await
                .map_err(|e| FrameworkError::SystemError(format!("LLM 调用失败: {}", e)))?;

            let raw = response.content.trim().to_string();
            let json_str = extract_json_from_response(&raw);

            match serde_json::from_str::<PartialBp>(&json_str) {
                Ok(partial) => {
                    let mut temp_bp = BlueprintJson::new("__validation__");
                    temp_bp.nodes = partial.nodes.clone();
                    temp_bp.connections = partial.connections.clone();

                    match BlueprintLoader::new()
                        .load_workflow_from_blueprint_json(temp_bp)
                        .await
                    {
                        Ok(_) => {
                            let mut final_bp = BlueprintJson::new(&workflow_name);
                            final_bp.metadata.description = workflow_desc.to_string();
                            final_bp.metadata.visibility = BlueprintVisibility::Private;
                            final_bp.nodes = partial.nodes;
                            final_bp.connections = partial.connections;

                            let node_count = final_bp.nodes.len();
                            let conn_count = final_bp.connections.len();

                            // 通过 World cache 获取 workflows_dir 并保存
                            let file_path_str = save_blueprint_via_world(ctx, &final_bp)
                                .await
                                .unwrap_or_else(|e| {
                                    tracing::warn!(
                                        "BuildWorkflowFromChain 保存失败 (non-fatal): {}",
                                        e
                                    );
                                    String::new()
                                });

                            tracing::debug!(
                                "BuildWorkflowFromChain 成功: {} 节点 {} 连线",
                                node_count,
                                conn_count
                            );
                            return Ok(AIOutput::success(
                                serde_json::json!({
                                    "node_count": node_count,
                                    "connection_count": conn_count,
                                    "file_path": file_path_str,
                                }),
                                format!(
                                    "工作流「{}」编译成功，共 {} 个节点、{} 条连线。已保存并注册。",
                                    workflow_name, node_count, conn_count
                                ),
                            ));
                        }
                        Err(e) => {
                            last_error = e.to_string();
                            error_context = format!("BlueprintLoader 编译失败：{}", e);
                            tracing::warn!(
                                "BuildWorkflowFromChain 第 {} 次 loader 失败: {}",
                                attempt,
                                e
                            );
                        }
                    }
                }
                Err(e) => {
                    last_error = e.to_string();
                    error_context = format!(
                        "JSON 解析失败：{}\n\n你输出的内容：\n{}",
                        e,
                        &raw[..raw.len().min(600)]
                    );
                    tracing::warn!("BuildWorkflowFromChain 第 {} 次解析失败: {}", attempt, e);
                }
            }
        }

        Ok(AIOutput::error(
            500,
            format!("工作流生成失败（重试 3 次），最后错误：{}", last_error),
        ))
    }

    fn name(&self) -> &str {
        build_system_names::BUILD_WORKFLOW_FROM_CHAIN
    }
}

// ============================================================================
// WfListWorkflows —— 列出已注册工作流（含 inputs/outputs 参数信息）
// ============================================================================

#[buns_system(
    "WfListWorkflows",
    description = "列出所有已注册工作流的名称、描述和参数定义",
    params {},
    destructive = false,
    readonly = true,
    idempotent = true,
    open_world = false
)]
pub struct WfListWorkflowsSystem;

#[async_trait]
impl SystemOperation for WfListWorkflowsSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        _input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let registry: Vec<crate::workflow::workflows::executor::BlueprintEntry> = ctx
            .get_world_cache()?
            .get_resource(crate::workflow::workflows::executor::REGISTRY)?
            .unwrap_or_default();

        if registry.is_empty() {
            return Ok(AIOutput::success(
                serde_json::json!({"workflows": [], "count": 0}),
                "当前系统中没有已注册的工作流。".to_string(),
            ));
        }

        let items: Vec<serde_json::Value> = registry
            .iter()
            .map(|e| {
                let inputs: Vec<serde_json::Value> = e
                    .metadata
                    .inputs
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "name": p.name,
                            "data_type": p.data_type,
                            "description": p.description,
                            "has_default": p.default_value.is_some(),
                        })
                    })
                    .collect();

                let outputs: Vec<serde_json::Value> = e
                    .metadata
                    .outputs
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "name": p.name,
                            "data_type": p.data_type,
                            "description": p.description,
                        })
                    })
                    .collect();

                serde_json::json!({
                    "name": e.metadata.name,
                    "description": e.metadata.description,
                    "source": format!("{:?}", e.source),
                    "inputs": inputs,
                    "outputs": outputs,
                })
            })
            .collect();

        let summary = registry
            .iter()
            .map(|e| {
                let src = match e.source {
                    crate::workflow::workflows::executor::WorkflowSource::Official => "官方",
                    crate::workflow::workflows::executor::WorkflowSource::Local => "本地",
                };
                let inputs_str = if e.metadata.inputs.is_empty() {
                    "无入参".to_string()
                } else {
                    e.metadata
                        .inputs
                        .iter()
                        .map(|p| format!("{}({})", p.name, p.data_type))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                format!(
                    "· {} [{}] — {} | 入参: {}",
                    e.metadata.name, src, e.metadata.description, inputs_str
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(AIOutput::success(
            serde_json::json!({"workflows": items, "count": registry.len()}),
            format!("已注册工作流（{}个）：\n{}", registry.len(), summary),
        ))
    }

    fn name(&self) -> &str {
        build_system_names::LIST_WORKFLOWS
    }
}

// ============================================================================
// WfRunWorkflow —— 按名称执行已注册工作流（即用即弃）
// ============================================================================

#[buns_system(
    "WfRunWorkflow",
    description = "执行{{name}}工作流，传入{{inputs}}参数",
    params {
        name:   "工作流名称（必填），与 WfListWorkflows 返回的 name 字段一致",
        inputs: "工作流入参（可选），JSON 对象格式，如 {\"source\":\"D:/music\",\"format\":\"mp3\"}。\
                 无入参时可省略"
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = true
)]
pub struct WfRunWorkflowSystem;

#[async_trait]
impl SystemOperation for WfRunWorkflowSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };

        let name = match args.safe_require("name") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };

        let inputs_raw: std::collections::HashMap<String, serde_json::Value> = args
            .get("inputs")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        // 从 registry 查找 file_path
        let registry: Vec<crate::workflow::workflows::executor::BlueprintEntry> = ctx
            .get_world_cache()?
            .get_resource(crate::workflow::workflows::executor::REGISTRY)?
            .unwrap_or_default();

        let entry = match registry.into_iter().find(|e| e.metadata.name == name) {
            Some(e) => e,
            None => {
                return Ok(AIOutput::error(
                    404,
                    format!(
                        "未找到名为「{}」的工作流，请先调用 WfListWorkflows 确认",
                        name
                    ),
                ))
            }
        };

        if entry.file_path.is_empty() {
            return Ok(AIOutput::error(
                404,
                format!("工作流「{}」无关联文件，无法执行", name),
            ));
        }

        // 即用即弃：从文件加载 → 创建实例 → 执行 → 丢弃
        let wf_inputs: std::collections::HashMap<String, DataValue> = inputs_raw
            .into_iter()
            .map(|(k, v)| (k, DataValue::new("JsonValue", v)))
            .collect();

        tracing::debug!(
            workflow_name = %name,
            file_path = %entry.file_path,
            "workflow tool execution started"
        );
        let t = std::time::Instant::now();

        let mut wf = BlueprintLoader::new()
            .load_workflow_from_file(&entry.file_path)
            .await
            .map_err(|e| FrameworkError::SystemError(format!("加载工作流失败: {}", e)))?;

        let outputs = wf
            .execute(wf_inputs)
            .await
            .map_err(|e| FrameworkError::SystemError(format!("执行失败: {}", e)))?;

        let duration_ms = t.elapsed().as_millis();

        let out_json: std::collections::HashMap<String, serde_json::Value> = outputs
            .into_iter()
            .map(|(k, v)| (k, v.json_value().clone()))
            .collect();

        let out_str =
            serde_json::to_string_pretty(&out_json).unwrap_or_else(|_| format!("{:?}", out_json));

        Ok(AIOutput::success(
            serde_json::json!({"outputs": out_json, "duration_ms": duration_ms}),
            format!(
                "工作流「{}」执行完成（{}ms）。\n输出：\n{}",
                name, duration_ms, out_str
            ),
        ))
    }

    fn name(&self) -> &str {
        build_system_names::RUN_WORKFLOW
    }
}

// ============================================================================
// WfRunScript —— 直接执行操作链文本（不保存，即用即弃）
// ============================================================================

#[buns_system(
    "WfRunScript",
    description = "直接编译并执行{{chain}}操作链脚本，传入{{inputs}}参数",
    params {
        chain:  "操作链文本（必填），如 \"1. $page_id = OpenBrowser(url=\\\"https://example.com\\\")\\n2. ClickElement(page_id=$page_id, selector=\\\"#btn\\\")\"",
        inputs: "工作流入参（可选），JSON 对象格式"
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = true
)]
pub struct WfRunScriptSystem;

#[async_trait]
impl SystemOperation for WfRunScriptSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };

        let chain = match args.safe_require("chain") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };

        let inputs_raw: std::collections::HashMap<String, serde_json::Value> = args
            .get("inputs")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        // 处理转义
        let chain_text = chain.replace(r"\n", "\n").replace(r#"\""#, "\"");

        if chain_text.trim().is_empty() {
            return Ok(AIOutput::error(400, "操作链文本不能为空。".to_string()));
        }

        let blueprint = corework::workflow::chain_compiler_v2::compile_chain_v2(&chain_text)
            .map_err(|e| {
                FrameworkError::SystemError(format!(
                    "操作链编译失败（第 {} 行）：{}",
                    e.line, e.message
                ))
            })?;

        // 验证可执行性 + 创建实例
        let wf_inputs: std::collections::HashMap<String, DataValue> = inputs_raw
            .into_iter()
            .map(|(k, v)| (k, DataValue::new("JsonValue", v)))
            .collect();

        tracing::debug!(
            "WfRunScript: 执行内联操作链（{} 行）",
            chain_text.lines().count()
        );
        let t = std::time::Instant::now();

        let mut wf = BlueprintLoader::new()
            .load_workflow_from_blueprint_json(blueprint)
            .await
            .map_err(|e| {
                FrameworkError::SystemError(format!(
                    "工作流实例化失败：{}\n\n操作链：\n{}",
                    e, chain_text
                ))
            })?;

        let outputs = wf
            .execute(wf_inputs)
            .await
            .map_err(|e| FrameworkError::SystemError(format!("脚本执行失败: {}", e)))?;

        let duration_ms = t.elapsed().as_millis();

        let out_json: std::collections::HashMap<String, serde_json::Value> = outputs
            .into_iter()
            .map(|(k, v)| (k, v.json_value().clone()))
            .collect();

        let out_str =
            serde_json::to_string_pretty(&out_json).unwrap_or_else(|_| format!("{:?}", out_json));

        Ok(AIOutput::success(
            serde_json::json!({"outputs": out_json, "duration_ms": duration_ms}),
            format!("脚本执行完成（{}ms）。\n输出：\n{}", duration_ms, out_str),
        ))
    }

    fn name(&self) -> &str {
        build_system_names::RUN_SCRIPT
    }
}

// ============================================================================
// WfReviseWorkflow —— 修改已有工作流
// ============================================================================

#[buns_system(
    "WfReviseWorkflow",
    description = "根据{{feedback}}修改{{name}}工作流并重新编译注册",
    params {
        name:     "要修改的工作流名称（必填）",
        feedback: "用户的修改意见（必填）"
    },
    destructive = false,
    readonly = false,
    idempotent = false,
    open_world = false
)]
pub struct WfReviseWorkflowSystem;

#[async_trait]
impl SystemOperation for WfReviseWorkflowSystem {
    type Input = AIInput;
    type Output = AIOutput;
    type Error = FrameworkError;

    async fn execute(
        &self,
        input: AIInput,
        ctx: &Context,
    ) -> std::result::Result<AIOutput, FrameworkError> {
        let args = match input.safe_parse_args() {
            Ok(a) => a,
            Err(e) => return Ok(e),
        };

        let name = match args.safe_require("name") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };
        let feedback = match args.safe_require("feedback") {
            Ok(v) => v,
            Err(e) => return Ok(e),
        };

        let registry: Vec<crate::workflow::workflows::executor::BlueprintEntry> = ctx
            .get_world_cache()?
            .get_resource(crate::workflow::workflows::executor::REGISTRY)?
            .unwrap_or_default();

        let entry = match registry.into_iter().find(|e| e.metadata.name == name) {
            Some(e) => e,
            None => {
                return Ok(AIOutput::error(
                    404,
                    format!("未找到名为「{}」的工作流", name),
                ))
            }
        };

        if entry.file_path.is_empty() {
            return Ok(AIOutput::error(
                404,
                format!("工作流「{}」无关联文件，无法修改", name),
            ));
        }

        if entry.source == crate::workflow::workflows::executor::WorkflowSource::Official {
            return Ok(AIOutput::error(
                403,
                format!(
                    "工作流「{}」是官方工作流，不可修改。如需调整请创建新的本地工作流。",
                    name
                ),
            ));
        }

        if entry.metadata.visibility == BlueprintVisibility::Public {
            return Ok(AIOutput::error(
                403,
                format!("工作流「{}」是公有工作流，不可修改。", name),
            ));
        }

        let original_json = tokio::fs::read_to_string(&entry.file_path)
            .await
            .map_err(|e| FrameworkError::SystemError(format!("读取工作流文件失败: {}", e)))?;

        let model: Option<String> = ctx.cache.get("model").await.unwrap_or(None);
        let model_name = model
            .as_deref()
            .filter(|m| *m != "default")
            .unwrap_or("qwen-plus");
        let model_uid = llm_gateway::key_store::find_by_name(model_name).ok_or_else(|| {
            FrameworkError::SystemError(format!(
                "未找到模型 '{}' 的 uid，请先在设置中配置",
                model_name
            ))
        })?;

        let system_prompt = crate::workflow::workflows::draft::build_revise_prompt(
            &name,
            &entry.metadata.description,
            &feedback,
        );

        #[derive(serde::Deserialize)]
        struct PartialBp {
            #[serde(default)]
            nodes: Vec<BlueprintNodeJson>,
            #[serde(default)]
            connections: Vec<ConnectionJson>,
        }

        let mut error_context = String::new();
        let mut last_error = String::new();

        for attempt in 1..=3u32 {
            let user_content = if error_context.is_empty() {
                format!(
                    "修改意见：{}\n\n原工作流 JSON：\n```json\n{}\n```\n\n请输出修改后的完整 nodes 和 connections。",
                    feedback, &original_json[..original_json.len().min(4000)]
                )
            } else {
                format!(
                    "修改意见：{}\n\n原工作流 JSON：\n```json\n{}\n```\n\n上次错误：\n{}",
                    feedback,
                    &original_json[..original_json.len().min(4000)],
                    error_context
                )
            };

            let messages = vec![
                llm_gateway::ChatMessage::system_cached(system_prompt.clone()),
                llm_gateway::ChatMessage {
                    role: "user".to_string(),
                    content: user_content,
                    cache_control: false,
                    tool_call_id: None,
                    name: None,
                    tool_calls: None,
                    reasoning_content: None,
                },
            ];

            let response = llm_gateway::call_llm(model_uid, &messages, None, None, None)
                .await
                .map_err(|e| FrameworkError::SystemError(format!("LLM 调用失败: {}", e)))?;

            let raw = response.content.trim().to_string();
            let json_str = extract_json_from_response(&raw);

            match serde_json::from_str::<PartialBp>(&json_str) {
                Ok(partial) => {
                    let mut temp_bp = BlueprintJson::new("__validation__");
                    temp_bp.nodes = partial.nodes.clone();
                    temp_bp.connections = partial.connections.clone();

                    match BlueprintLoader::new()
                        .load_workflow_from_blueprint_json(temp_bp)
                        .await
                    {
                        Ok(_) => {
                            let mut final_bp = BlueprintJson::from_json_str(&original_json)
                                .unwrap_or_else(|_| BlueprintJson::new(&name));
                            final_bp.nodes = partial.nodes;
                            final_bp.connections = partial.connections;

                            // 保存回原文件
                            let new_json = serde_json::to_string_pretty(&final_bp)
                                .map_err(|e| FrameworkError::SystemError(e.to_string()))?;
                            tokio::fs::write(&entry.file_path, &new_json)
                                .await
                                .map_err(|e| {
                                    FrameworkError::SystemError(format!("写回文件失败: {}", e))
                                })?;

                            // 更新 registry 中的 metadata
                            if let Ok(world) = ctx.get_world_cache() {
                                let mut reg: Vec<
                                    crate::workflow::workflows::executor::BlueprintEntry,
                                > = world
                                    .get_resource(crate::workflow::workflows::executor::REGISTRY)?
                                    .unwrap_or_default();
                                if let Some(e) = reg.iter_mut().find(|e| e.metadata.name == name) {
                                    e.metadata = final_bp.metadata.clone();
                                }
                                let _ = world.set_resource(
                                    crate::workflow::workflows::executor::REGISTRY,
                                    &reg,
                                    None,
                                );
                            }

                            tracing::debug!(workflow_name = %name, "workflow revised");
                            return Ok(AIOutput::success(
                                serde_json::json!({"name": name, "file": entry.file_path}),
                                format!(
                                    "工作流「{}」已按修改意见更新并保存。下次执行将使用新版本。",
                                    name
                                ),
                            ));
                        }
                        Err(e) => {
                            last_error = e.to_string();
                            error_context = format!("BlueprintLoader 编译失败：{}", e);
                            tracing::warn!("WfRevise 第 {} 次 loader 失败: {}", attempt, e);
                        }
                    }
                }
                Err(e) => {
                    last_error = e.to_string();
                    error_context = format!("JSON 解析失败：{}", e);
                }
            }
        }

        Ok(AIOutput::error(
            500,
            format!("工作流修改失败（重试 3 次），最后错误：{}", last_error),
        ))
    }

    fn name(&self) -> &str {
        build_system_names::REVISE_WORKFLOW
    }
}

// ============================================================================
// 内部工具函数
// ============================================================================

/// 从 LLM 响应中提取 JSON 字符串
fn extract_json_from_response(raw: &str) -> String {
    let stripped = if let Some(start) = raw.find("```json") {
        let content = &raw[start + 7..];
        if let Some(end) = content.find("```") {
            content[..end].trim().to_string()
        } else {
            content.trim().to_string()
        }
    } else if let Some(start) = raw.find("```") {
        let content = &raw[start + 3..];
        if let Some(end) = content.find("```") {
            content[..end].trim().to_string()
        } else {
            content.trim().to_string()
        }
    } else {
        raw.to_string()
    };

    if stripped.starts_with('{') {
        stripped
    } else if let Some(brace_pos) = stripped.find('{') {
        stripped[brace_pos..].to_string()
    } else {
        stripped
    }
}

/// 通过 World cache 获取 workflows_dir 并保存 BlueprintJson。
/// 返回保存后的文件路径。
async fn save_blueprint_via_world(
    ctx: &Context,
    blueprint: &BlueprintJson,
) -> std::result::Result<String, FrameworkError> {
    let mut blueprint = blueprint.clone();
    if blueprint.metadata.id.is_empty() {
        blueprint.metadata.id = blueprint.metadata.name.clone();
    }
    blueprint.normalize_node_sizes();
    let world = ctx.get_world_cache()?;

    // 读取 workflows_dir（由 lib.rs setup 阶段写入 World cache）
    let workflows_dir: String = world
        .get_resource("wf:workflows_dir")?
        .ok_or_else(|| FrameworkError::SystemError("wf:workflows_dir 未设置".into()))?;

    let safe_name = blueprint
        .metadata
        .name
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let file_name = format!("{}.workflow.json", safe_name);
    let save_path = std::path::Path::new(&workflows_dir).join(&file_name);

    let json_pretty = serde_json::to_string_pretty(&blueprint)
        .map_err(|e| FrameworkError::SystemError(format!("序列化失败: {}", e)))?;

    if let Some(parent) = save_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    tokio::fs::write(&save_path, &json_pretty)
        .await
        .map_err(|e| FrameworkError::SystemError(format!("保存文件失败: {}", e)))?;

    let file_path_str = save_path.to_string_lossy().to_string();

    // 注册到 registry
    let mut registry: Vec<crate::workflow::workflows::executor::BlueprintEntry> = world
        .get_resource(crate::workflow::workflows::executor::REGISTRY)?
        .unwrap_or_default();
    registry.retain(|e| e.metadata.name != blueprint.metadata.name);
    registry.push(crate::workflow::workflows::executor::BlueprintEntry {
        metadata: blueprint.metadata.clone(),
        file_path: file_path_str.clone(),
        key: format!("{}:auto", blueprint.metadata.name),
        source: crate::workflow::workflows::executor::WorkflowSource::Local,
    });
    let _ = world.set_resource(
        crate::workflow::workflows::executor::REGISTRY,
        &registry,
        None,
    );

    tracing::debug!(
        "save_blueprint_via_world: {} → {}",
        blueprint.metadata.name,
        file_path_str
    );
    Ok(file_path_str)
}
