//! 蓝图加载器
//!
//! 将 JSON 格式的蓝图转换为可执行的 Blueprint

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::error::{FrameworkError, Result};
use crate::workflow::builder::{BlueprintBuilder, CompiledBlueprint};
use crate::workflow::core::DataValue;
use crate::workflow::execution::WorkflowSourceRef;
use crate::workflow::nodes::control::PinMapping;
use crate::workflow::nodes::data::variable::GetVarNode;
use crate::workflow::registry::{NodePermissions, NodeRegistry, PinKind};
use crate::workflow::DynamicSystemNode;

use super::blueprint_json::{BlueprintJson, BlueprintMetadata, BlueprintNodeJson, ConnectionJson};

/// 蓝图加载结果 - 同时包含元数据和编译后的实例
#[derive(Debug, Clone)]
pub struct LoadedBlueprint {
    /// 蓝图元数据
    pub metadata: BlueprintMetadata,

    /// 编译后的蓝图实例
    pub compiled: CompiledBlueprint,
}

/// 蓝图加载器
pub struct BlueprintLoader;

impl BlueprintLoader {
    /// 创建新的加载器
    pub fn new() -> Self {
        Self
    }

    /// 从 JSON 字符串加载蓝图（返回元数据和实例）
    pub fn load_from_json_str(
        &self,
        json_str: &str,
        ctx: &crate::orchestration::Context,
    ) -> Result<LoadedBlueprint> {
        let blueprint_json = BlueprintJson::from_json_str(json_str)
            .map_err(|e| FrameworkError::WorkflowError(format!("JSON 解析失败: {}", e)))?;

        self.load_from_blueprint_json(blueprint_json, ctx)
    }

    /// 从 JSON 字符串加载蓝图（仅返回编译实例，向后兼容）
    pub fn load_compiled_from_json_str(
        &self,
        json_str: &str,
        ctx: &crate::orchestration::Context,
    ) -> Result<CompiledBlueprint> {
        let loaded = self.load_from_json_str(json_str, ctx)?;
        Ok(loaded.compiled)
    }

    /// 从文件加载蓝图（返回元数据和实例）
    pub fn load_from_file(
        &self,
        path: impl AsRef<Path>,
        ctx: &crate::orchestration::Context,
    ) -> Result<LoadedBlueprint> {
        let json_str = std::fs::read_to_string(path.as_ref())
            .map_err(|e| FrameworkError::WorkflowError(format!("读取文件失败: {}", e)))?;

        self.load_from_json_str(&json_str, ctx)
    }

    /// 从文件加载蓝图（仅返回编译实例，向后兼容）
    pub fn load_compiled_from_file(
        &self,
        path: impl AsRef<Path>,
        ctx: &crate::orchestration::Context,
    ) -> Result<CompiledBlueprint> {
        let loaded = self.load_from_file(path, ctx)?;
        Ok(loaded.compiled)
    }

    /// 从 JSON 字符串加载为可复用的Workflow
    pub async fn load_workflow_from_json_str(
        &self,
        json_str: &str,
    ) -> Result<crate::workflow::workflow::Workflow> {
        let blueprint_json = BlueprintJson::from_json_str(json_str)
            .map_err(|e| FrameworkError::WorkflowError(format!("JSON 解析失败: {}", e)))?;

        self.load_workflow_from_blueprint_json(blueprint_json).await
    }

    /// 从文件加载为可复用的Workflow
    pub async fn load_workflow_from_file(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<crate::workflow::workflow::Workflow> {
        let json_str = std::fs::read_to_string(path.as_ref())
            .map_err(|e| FrameworkError::WorkflowError(format!("读取文件失败: {}", e)))?;

        self.load_workflow_from_json_str(&json_str).await
    }

    /// 从 BlueprintJson 加载蓝图（返回元数据和实例）
    pub fn load_from_blueprint_json(
        &self,
        mut blueprint_json: BlueprintJson,
        ctx: &crate::orchestration::Context,
    ) -> Result<LoadedBlueprint> {
        blueprint_json.normalize_node_sizes();
        // 验证蓝图结构
        blueprint_json
            .validate()
            .map_err(|e| FrameworkError::WorkflowError(format!("蓝图验证失败: {}", e)))?;

        // 提取元数据（克隆）
        let metadata = blueprint_json.metadata.clone();

        // 创建 BlueprintBuilder（像 batch_grading_blueprint.rs 那样）
        let mut builder = BlueprintBuilder::new(&blueprint_json.metadata.name);

        // 节点 ID -> 节点名称映射
        let mut node_id_to_name: HashMap<String, String> = HashMap::new();

        // 节点 ID -> 节点类型映射（用于连接时引脚权限验证）
        let mut node_id_to_type: HashMap<String, String> = HashMap::new();

        // 存储每个节点的 split pin 映射：(node_id, pin_name) -> { full_pin_name -> cache_key }
        let split_pin_mappings: HashMap<(String, String), HashMap<String, String>> = HashMap::new();

        // 第一步：找到 StartNode 和 EndNode
        let mut start_node = None;
        let mut end_node = None;
        let mut other_nodes = Vec::new();

        for node_json in &blueprint_json.nodes {
            match node_json.node_type.as_str() {
                "StartNode" => start_node = Some(node_json),
                "EndNode" => end_node = Some(node_json),
                _ => other_nodes.push(node_json),
            }
        }

        // 添加 Start 节点
        let _start_name = if let Some(start) = start_node {
            let name = start
                .display_name
                .as_ref()
                .filter(|s| !s.is_empty()) // 过滤空字符串
                .map(|s| s.as_str())
                .unwrap_or("Start");
            tracing::debug!("🎬 添加StartNode: '{}' (id: {})", name, start.id);

            // 从 pins 构建 output mappings（DataOutput引脚）
            let output_mappings: Vec<PinMapping> = start
                .pins
                .iter()
                .filter(|pin| pin.kind == "DataOutput")
                .map(|pin| {
                    // StartNode的output cache_key用 "{entry_name}:{pin_name}" 格式，
                    // 与 execute_with_params 写入格式保持一致
                    PinMapping::new(&pin.name, format!("{}:{}", name, &pin.name), &pin.data_type)
                })
                .collect();

            if output_mappings.is_empty() {
                builder = builder.add_start(name);
            } else {
                builder = builder.add_start_with_outputs(name, output_mappings);
            }

            node_id_to_name.insert(start.id.clone(), name.to_string());
            node_id_to_type.insert(start.id.clone(), "StartNode".to_string());
            name.to_string()
        } else {
            // 如果没有 StartNode，自动添加
            builder = builder.add_start("Start");
            "Start".to_string()
        };

        // 添加 End 节点
        let _end_name = if let Some(end) = end_node {
            let name = end
                .display_name
                .as_ref()
                .filter(|s| !s.is_empty()) // 过滤空字符串
                .map(|s| s.as_str())
                .unwrap_or("End");

            // 从 pins 构建 PinMapping
            let input_mappings: Vec<PinMapping> = end
                .pins
                .iter()
                .filter(|pin| pin.kind == "DataInput")
                .map(|pin| {
                    // 使用 pin.name 作为 cache_key
                    PinMapping::new(&pin.name, &pin.name, &pin.data_type)
                })
                .collect();

            if input_mappings.is_empty() {
                builder = builder.add_end(name);
            } else {
                builder = builder.add_end_with_inputs(name, input_mappings);
            }

            node_id_to_name.insert(end.id.clone(), name.to_string());
            node_id_to_type.insert(end.id.clone(), "EndNode".to_string());
            name.to_string()
        } else {
            builder = builder.add_end("End");
            "End".to_string()
        };

        // 第二步：添加其他节点
        for node_json in other_nodes {
            let node_name =
                self.add_node_to_builder(&mut builder, node_json, &blueprint_json, ctx)?;
            tracing::debug!(
                "🔧 添加节点: '{}' (id: {}, type: {})",
                node_name,
                node_json.id,
                node_json.node_type
            );
            node_id_to_name.insert(node_json.id.clone(), node_name);
            node_id_to_type.insert(node_json.id.clone(), node_json.node_type.clone());
        }

        // 第三步：添加连接（处理 split pin 映射）
        for conn in &blueprint_json.connections {
            let source_name = node_id_to_name.get(&conn.source_node).ok_or_else(|| {
                FrameworkError::WorkflowError(format!("连接的源节点不存在: {}", conn.source_node))
            })?;
            let target_name = node_id_to_name.get(&conn.target_node).ok_or_else(|| {
                FrameworkError::WorkflowError(format!("连接的目标节点不存在: {}", conn.target_node))
            })?;

            // 基于权限的引脚名称严格验证
            self.validate_connection_pins(conn, &node_id_to_type)?;

            tracing::debug!(
                "🔗 添加连接: {} [{}] -> {} [{}]",
                source_name,
                conn.source_pin,
                target_name,
                conn.target_pin
            );

            // 解析源 pin（可能是 split pin 的子字段，如 "position.x"）
            let source_pin =
                self.resolve_split_pin(&conn.source_node, &conn.source_pin, &split_pin_mappings);

            // 解析目标 pin
            let target_pin =
                self.resolve_split_pin(&conn.target_node, &conn.target_pin, &split_pin_mappings);

            builder = builder.connect(source_name, &source_pin, target_name, &target_pin);
        }

        // 第四步：编译
        let mut compiled = builder.compile()?;
        compiled.source_map = Self::extract_source_map(&blueprint_json.nodes, &node_id_to_name);

        // 第五步：解析节点的default_value作为默认值
        // 从pins的default_value字段中提取，而不是properties
        for node_json in &blueprint_json.nodes {
            if let Some(node_name) = node_id_to_name.get(&node_json.id) {
                let mut pin_defaults = HashMap::new();

                // 遍历所有pins，收集有default_value的数据输入引脚
                for pin in &node_json.pins {
                    // 只处理数据输入引脚（含默认值）
                    if pin.kind != "DataInput" {
                        continue;
                    }

                    // 检查是否有default_value
                    if let Some(default_val) = &pin.default_value {
                        // 如果不是null或空值
                        if !default_val.is_null() {
                            if let Ok(data_value) =
                                self.json_value_to_data_value(default_val, &pin.data_type)
                            {
                                tracing::debug!(
                                    "  📌 从pin {}={:?} 提取默认值 ({})",
                                    pin.name,
                                    default_val,
                                    pin.data_type
                                );
                                pin_defaults.insert(pin.name.clone(), data_value);
                            }
                        }
                    }
                }

                // 如果有默认值，保存到node_defaults
                if !pin_defaults.is_empty() {
                    tracing::debug!(
                        "✅ 节点 '{}' 加载了 {} 个默认值",
                        node_name,
                        pin_defaults.len()
                    );
                    compiled
                        .node_defaults
                        .insert(node_name.clone(), pin_defaults);
                }
            }
        }

        compiled.variable_declarations =
            Self::collect_workflow_variable_declarations(&blueprint_json);

        for variable in &blueprint_json.variables {
            let Some(default_value) = &variable.default_value else {
                continue;
            };
            let value = self.json_value_to_data_value(default_value, &variable.data_type)?;
            compiled
                .variable_defaults
                .insert(variable.name.clone(), value);
        }

        Ok(LoadedBlueprint { metadata, compiled })
    }

    fn collect_workflow_variable_declarations(blueprint_json: &BlueprintJson) -> HashSet<String> {
        let mut declarations = HashSet::new();
        for variable in &blueprint_json.variables {
            declarations.insert(variable.name.clone());
        }
        for input in &blueprint_json.metadata.inputs {
            declarations.insert(input.name.clone());
        }
        for node in &blueprint_json.nodes {
            if node.node_type != "StartNode" {
                continue;
            }
            for pin in &node.pins {
                if pin.kind == "DataOutput" {
                    declarations.insert(pin.name.clone());
                }
            }
        }
        declarations
    }

    fn extract_source_map(
        nodes: &[BlueprintNodeJson],
        node_id_to_name: &HashMap<String, String>,
    ) -> HashMap<String, WorkflowSourceRef> {
        let mut source_map = HashMap::new();
        for node in nodes {
            let Some(runtime_name) = node_id_to_name.get(&node.id) else {
                continue;
            };
            let Some(source_value) = node.properties.get("source_script") else {
                continue;
            };
            if let Some(source_ref) = WorkflowSourceRef::from_json(source_value) {
                source_map.insert(runtime_name.clone(), source_ref);
            }
        }
        source_map
    }

    /// 从 BlueprintJson 加载为可复用的Workflow（简化实现，复用Builder）
    ///
    /// 注意：当前实现无法加载节点默认值，因为需要Context来创建DynamicSystemNode。
    /// 如果需要默认值，请使用`load_from_blueprint_json`然后手动转换。
    pub async fn load_workflow_from_blueprint_json(
        &self,
        mut blueprint_json: BlueprintJson,
    ) -> Result<crate::workflow::workflow::Workflow> {
        use crate::workflow::workflow::PinDefinition;

        blueprint_json.normalize_node_sizes();
        blueprint_json
            .validate()
            .map_err(|e| FrameworkError::WorkflowError(format!("蓝图验证失败: {}", e)))?;

        // 创建 BlueprintBuilder（简化版本，只支持基础节点，不创建DynamicSystemNode）
        let mut builder = BlueprintBuilder::new(&blueprint_json.metadata.name);
        let _workflow_name = blueprint_json.metadata.name.clone();

        // 节点 ID -> 节点名称映射
        let mut node_id_to_name: HashMap<String, String> = HashMap::new();

        // 第一步：找到 StartNode 和 EndNode
        let mut start_node_json = None;
        let mut end_node_json = None;

        for node_json in &blueprint_json.nodes {
            match node_json.node_type.as_str() {
                "StartNode" => start_node_json = Some(node_json),
                "EndNode" => end_node_json = Some(node_json),
                _ => {} // 暂不支持其他节点
            }
        }

        // 添加 Start 节点 + 收集input_pins
        let mut input_pins = Vec::new();
        let _start_name = if let Some(start) = start_node_json {
            let name = start
                .display_name
                .as_ref()
                .filter(|s| !s.is_empty())
                .map(|s| s.as_str())
                .unwrap_or("Start");

            // 从 pins 构建 output mappings + input_pins定义
            let output_mappings: Vec<PinMapping> = start
                .pins
                .iter()
                .filter(|pin| pin.kind == "DataOutput")
                .map(|pin| {
                    // 同时收集到input_pins
                    input_pins.push(PinDefinition {
                        name: pin.name.clone(),
                        data_type: pin.data_type.clone(),
                        description: pin.description.clone(),
                        default_value: pin
                            .default_value
                            .as_ref()
                            .and_then(|v| self.json_value_to_data_value(v, &pin.data_type).ok()),
                    });

                    PinMapping::new(&pin.name, format!("{}:{}", name, &pin.name), &pin.data_type)
                })
                .collect();

            if output_mappings.is_empty() {
                builder = builder.add_start(name);
            } else {
                builder = builder.add_start_with_outputs(name, output_mappings);
            }

            node_id_to_name.insert(start.id.clone(), name.to_string());
            name.to_string()
        } else {
            builder = builder.add_start("Start");
            "Start".to_string()
        };

        // 添加 End 节点 + 收集output_pins
        let mut output_pins = Vec::new();
        let _end_name = if let Some(end) = end_node_json {
            let name = end
                .display_name
                .as_ref()
                .filter(|s| !s.is_empty())
                .map(|s| s.as_str())
                .unwrap_or("End");

            // 从 pins 构建 PinMapping + output_pins定义
            let input_mappings: Vec<PinMapping> = end
                .pins
                .iter()
                .filter(|pin| pin.kind == "DataInput")
                .map(|pin| {
                    // 同时收集到output_pins
                    output_pins.push(PinDefinition {
                        name: pin.name.clone(),
                        data_type: pin.data_type.clone(),
                        description: pin.description.clone(),
                        default_value: None, // EndNode不需要默认值
                    });

                    PinMapping::new(&pin.name, &pin.name, &pin.data_type)
                })
                .collect();

            if input_mappings.is_empty() {
                builder = builder.add_end(name);
            } else {
                builder = builder.add_end_with_inputs(name, input_mappings);
            }

            node_id_to_name.insert(end.id.clone(), name.to_string());
            name.to_string()
        } else {
            builder = builder.add_end("End");
            "End".to_string()
        };

        // 第二步：添加连接（暂时只支持Start->End的简单连接）
        for conn in &blueprint_json.connections {
            if let (Some(source_name), Some(target_name)) = (
                node_id_to_name.get(&conn.source_node),
                node_id_to_name.get(&conn.target_node),
            ) {
                builder =
                    builder.connect(source_name, &conn.source_pin, target_name, &conn.target_pin);
            }
        }

        // 第三步：收集默认值
        let mut default_values = HashMap::new();
        for node_json in &blueprint_json.nodes {
            if let Some(_node_name) = node_id_to_name.get(&node_json.id) {
                for pin in &node_json.pins {
                    if pin.kind == "DataInput" {
                        if let Some(default_val) = &pin.default_value {
                            if !default_val.is_null() {
                                if let Ok(data_value) =
                                    self.json_value_to_data_value(default_val, &pin.data_type)
                                {
                                    // 使用pin.name作为cache_key
                                    default_values.insert(pin.name.clone(), data_value);
                                }
                            }
                        }
                    }
                }
            }
        }

        // 构建Workflow
        builder.build().await
    }

    /// 基于权限的连接引脚严格验证
    ///
    /// - 对没有 `CAN_ADD_OUTPUT_PIN` 的节点：验证 `source_pin` 在注册的输出引脚中存在
    /// - 对没有 `CAN_ADD_INPUT_PIN` 的节点：验证 `target_pin` 在注册的输入引脚中存在
    /// - StartNode / EndNode 的引脚来自 JSON（动态），跳过验证
    fn validate_connection_pins(
        &self,
        conn: &ConnectionJson,
        node_id_to_type: &HashMap<String, String>,
    ) -> Result<()> {
        // ── 源节点输出引脚验证 ──
        if let Some(src_type) = node_id_to_type.get(&conn.source_node) {
            if src_type != "StartNode" && src_type != "EndNode" {
                if let Some(meta) = NodeRegistry::get(src_type) {
                    if !meta.permissions.has(NodePermissions::CAN_ADD_OUTPUT_PIN) {
                        let valid: Vec<&str> = meta
                            .pins
                            .iter()
                            .filter(|p| matches!(p.kind, PinKind::ExecOutput | PinKind::DataOutput))
                            .map(|p| p.name)
                            .collect();
                        if !valid.contains(&conn.source_pin.as_str()) {
                            return Err(FrameworkError::WorkflowError(format!(
                                "引脚验证失败: 节点 '{}' (类型: {}) 没有输出引脚 '{}', 可用输出引脚: {:?}",
                                conn.source_node, src_type, conn.source_pin, valid
                            )));
                        }
                    }
                }
            }
        }

        // ── 目标节点输入引脚验证 ──
        if let Some(tgt_type) = node_id_to_type.get(&conn.target_node) {
            if tgt_type != "StartNode" && tgt_type != "EndNode" {
                if let Some(meta) = NodeRegistry::get(tgt_type) {
                    if !meta.permissions.has(NodePermissions::CAN_ADD_INPUT_PIN) {
                        let valid: Vec<&str> = meta
                            .pins
                            .iter()
                            .filter(|p| matches!(p.kind, PinKind::ExecInput | PinKind::DataInput))
                            .map(|p| p.name)
                            .collect();
                        if !valid.contains(&conn.target_pin.as_str()) {
                            return Err(FrameworkError::WorkflowError(format!(
                                "引脚验证失败: 节点 '{}' (类型: {}) 没有输入引脚 '{}', 可用输入引脚: {:?}",
                                conn.target_node, tgt_type, conn.target_pin, valid
                            )));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// 解析可能的 split pin 名称
    /// 如果是 split pin 的子字段（如 "position.x"），返回实际的 cache key
    /// 否则返回原 pin 名称
    fn resolve_split_pin(
        &self,
        node_id: &str,
        pin_name: &str,
        split_pin_mappings: &HashMap<(String, String), HashMap<String, String>>,
    ) -> String {
        // 检查是否是 split pin 的子字段（包含 '.'）
        if let Some(dot_pos) = pin_name.find('.') {
            let parent_pin = &pin_name[..dot_pos];

            // 查找是否有该 pin 的 split 配置
            if let Some(mapping) =
                split_pin_mappings.get(&(node_id.to_string(), parent_pin.to_string()))
            {
                // 使用映射表获取实际的 cache key (sub_pin_name)
                if let Some(cache_key) = mapping.get(pin_name) {
                    return cache_key.to_string();
                }
            }
        }

        // 没有 split 配置，直接返回原名称
        pin_name.to_string()
    }

    /// 添加节点到 builder
    fn add_node_to_builder(
        &self,
        builder: &mut BlueprintBuilder,
        node_json: &BlueprintNodeJson,
        blueprint_json: &BlueprintJson,
        ctx: &crate::orchestration::Context,
    ) -> Result<String> {
        // 节点名称（用于连接）- 先于注册表检查计算，以便错误信息中包含可读名称
        let node_name = node_json
            .display_name
            .as_ref()
            .filter(|s| !s.is_empty()) // 过滤空字符串
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                let id_prefix = &node_json.id[..std::cmp::min(8, node_json.id.len())];
                format!("{}_{}", node_json.node_type, id_prefix)
            });

        // 查找节点元数据（同时作存在性验证）
        let node_meta = if let Some(node_meta) = NodeRegistry::get(&node_json.node_type) {
            node_meta
        } else {
            let metadata = ctx
                .get_registry()
                .get_dynamic_metadata(&node_json.node_type)
                .ok_or_else(|| {
                    FrameworkError::WorkflowError(format!(
                        "不支持的节点 '{}' (类型: {})",
                        node_name, node_json.node_type
                    ))
                })?;

            let mut dynamic_builder = DynamicSystemNode::builder(&node_name, &node_json.node_type);
            for parameter in &metadata.parameters {
                dynamic_builder = dynamic_builder.map_input_with_field(
                    &parameter.name,
                    &parameter.name,
                    format!("{}:{}", node_name, parameter.name),
                    &parameter.param_type,
                );
            }
            for output in &metadata.outputs {
                dynamic_builder = dynamic_builder.map_output_with_field(
                    &output.name,
                    &output.name,
                    format!("{}:{}", node_name, output.name),
                    &output.field_type,
                );
            }
            let dynamic_node = dynamic_builder.build();
            let temp_builder = std::mem::replace(builder, BlueprintBuilder::new(""));
            *builder = temp_builder.add_dynamic_node(&node_name, std::sync::Arc::new(dynamic_node));
            return Ok(node_name);
        };

        if node_json.node_type == "SetVarNode" {
            Self::validate_static_set_var_reference(blueprint_json, node_json)?;
        }

        if node_json.node_type == "GetVarNode" {
            let variable_name = Self::get_var_node_variable_name(blueprint_json, node_json)?;
            if let Some(variable_name) = variable_name.as_deref() {
                Self::validate_get_var_reference(blueprint_json, variable_name)?;
            }
            let temp_builder = std::mem::replace(builder, BlueprintBuilder::new(""));
            *builder = temp_builder.add_impure_node(
                &node_name,
                GetVarNode::with_instance(&node_name, variable_name.unwrap_or_default()),
            );
            return Ok(node_name);
        }

        // 对含 CAN_ADD_OUTPUT_PIN 权限的节点，从 JSON pins 动态构建
        if node_meta
            .permissions
            .has(NodePermissions::CAN_ADD_OUTPUT_PIN)
        {
            let exec_out_pins: Vec<String> = node_json
                .pins
                .iter()
                .filter(|p| p.kind == "ExecOutput")
                .map(|p| p.name.clone())
                .collect();

            if let Some(node) = NodeRegistry::create_node_with_exec_pins(
                &node_json.node_type,
                &node_name,
                exec_out_pins,
            ) {
                let temp_builder = std::mem::replace(builder, BlueprintBuilder::new(""));
                *builder = temp_builder.add_dynamic_node(&node_name, node);
                return Ok(node_name);
            }
        }

        // 普通节点：通过 Context 创建实例
        if let Ok(node) = ctx.create_node_by_type(&node_json.node_type, &node_name) {
            let temp_builder = std::mem::replace(builder, BlueprintBuilder::new(""));
            *builder = temp_builder.add_dynamic_node(&node_name, node);
            return Ok(node_name);
        }

        Err(FrameworkError::WorkflowError(format!(
            "不支持的节点 '{}' (类型: {})",
            node_name, node_json.node_type
        )))
    }

    fn get_var_node_variable_name(
        blueprint_json: &BlueprintJson,
        node_json: &BlueprintNodeJson,
    ) -> Result<Option<String>> {
        let name_pin = node_json
            .pins
            .iter()
            .find(|pin| pin.name == "Name" && pin.kind == "DataInput");
        let has_name_connection = blueprint_json.connections.iter().any(|connection| {
            connection.target_node == node_json.id && connection.target_pin == "Name"
        });
        if has_name_connection {
            return Ok(None);
        }
        let value = name_pin
            .and_then(|pin| pin.default_value.as_ref())
            .or_else(|| node_json.properties.get("variable_name"))
            .ok_or_else(|| {
                FrameworkError::WorkflowError(format!(
                    "GetVarNode '{}' 的 Name 引脚必须填写或连接",
                    node_json.id
                ))
            })?;
        let name = value
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                FrameworkError::WorkflowError(format!(
                    "GetVarNode '{}' 的 Name 引脚必须是非空字符串",
                    node_json.id
                ))
            })?;
        Ok(Some(name.to_string()))
    }

    fn validate_static_set_var_reference(
        blueprint_json: &BlueprintJson,
        node_json: &BlueprintNodeJson,
    ) -> Result<()> {
        let Some(name_pin) = node_json
            .pins
            .iter()
            .find(|pin| pin.name == "Name" && pin.kind == "DataInput")
        else {
            return Ok(());
        };
        let Some(default_value) = &name_pin.default_value else {
            return Ok(());
        };
        let Some(name) = default_value
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return Ok(());
        };
        Self::validate_get_var_reference(blueprint_json, name).map_err(|_| {
            FrameworkError::WorkflowError(format!(
                "SetVarNode can only write declared workflow variables or inputs, not '${}'",
                name
            ))
        })
    }

    fn validate_get_var_reference(blueprint_json: &BlueprintJson, name: &str) -> Result<()> {
        let in_variables = blueprint_json
            .variables
            .iter()
            .any(|variable| variable.name == name);
        let in_inputs = blueprint_json
            .metadata
            .inputs
            .iter()
            .any(|input| input.name == name);
        let in_start_outputs = blueprint_json.nodes.iter().any(|node| {
            node.node_type == "StartNode"
                && node
                    .pins
                    .iter()
                    .any(|pin| pin.kind == "DataOutput" && pin.name == name)
        });
        if in_variables || in_inputs || in_start_outputs {
            return Ok(());
        }

        Err(FrameworkError::WorkflowError(format!(
            "GetVarNode 只能引用已声明的 workflow input 或变量，未找到: ${}",
            name
        )))
    }

    /// 将JSON值转换为DataValue
    fn json_value_to_data_value(
        &self,
        value: &serde_json::Value,
        data_type: &str,
    ) -> Result<DataValue> {
        // 根据指定的data_type进行类型转换
        match data_type {
            "i64" => {
                // 如果JSON是数字，直接用
                if let Some(i) = value.as_i64() {
                    return Ok(DataValue::from_i64(i));
                }
                // 如果JSON是字符串，尝试解析
                if let Some(s) = value.as_str() {
                    if let Ok(i) = s.parse::<i64>() {
                        return Ok(DataValue::from_i64(i));
                    }
                }
                Err(FrameworkError::WorkflowError(format!(
                    "无法将 {:?} 转换为 i64",
                    value
                )))
            }
            "f64" => {
                // 如果JSON是数字，直接用
                if let Some(f) = value.as_f64() {
                    return Ok(DataValue::from_f64(f));
                }
                // 如果JSON是字符串，尝试解析
                if let Some(s) = value.as_str() {
                    if let Ok(f) = s.parse::<f64>() {
                        return Ok(DataValue::from_f64(f));
                    }
                }
                Err(FrameworkError::WorkflowError(format!(
                    "无法将 {:?} 转换为 f64",
                    value
                )))
            }
            "String" | "str" => {
                // 如果JSON是字符串，直接用
                if let Some(s) = value.as_str() {
                    return Ok(DataValue::from_string(s.to_string()));
                }
                // 其他类型转换为字符串
                let json_str = serde_json::to_string(value)
                    .map_err(|e| FrameworkError::WorkflowError(format!("JSON序列化失败: {}", e)))?;
                Ok(DataValue::from_string(json_str))
            }
            "bool" => {
                // 如果JSON是bool，直接用
                if let Some(b) = value.as_bool() {
                    return Ok(DataValue::from_bool(b));
                }
                // 如果JSON是字符串，尝试解析
                if let Some(s) = value.as_str() {
                    match s.to_lowercase().as_str() {
                        "true" | "1" | "yes" => return Ok(DataValue::from_bool(true)),
                        "false" | "0" | "no" => return Ok(DataValue::from_bool(false)),
                        _ => {}
                    }
                }
                Err(FrameworkError::WorkflowError(format!(
                    "无法将 {:?} 转换为 bool",
                    value
                )))
            }
            // 数组类型
            dt if dt.starts_with("Array<") || dt.starts_with("Vec<") => {
                // 数组必须是JSON数组
                if value.is_array() {
                    let json_str = serde_json::to_string(value).map_err(|e| {
                        FrameworkError::WorkflowError(format!("JSON序列化失败: {}", e))
                    })?;
                    Ok(DataValue::from_string(json_str))
                } else if let Some(s) = value.as_str() {
                    // 或者是JSON字符串表示的数组
                    Ok(DataValue::from_string(s.to_string()))
                } else {
                    Err(FrameworkError::WorkflowError(format!(
                        "无法将 {:?} 转换为数组类型 {}",
                        value, dt
                    )))
                }
            }
            _ => {
                // 未知类型，按通用逻辑处理
                match value {
                    serde_json::Value::Bool(b) => Ok(DataValue::from_bool(*b)),
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Ok(DataValue::from_i64(i))
                        } else if let Some(f) = n.as_f64() {
                            Ok(DataValue::from_f64(f))
                        } else {
                            Err(FrameworkError::WorkflowError(format!(
                                "无法解析数字: {}",
                                n
                            )))
                        }
                    }
                    serde_json::Value::String(s) => Ok(DataValue::from_string(s.clone())),
                    serde_json::Value::Null => Ok(DataValue::from_string(String::new())),
                    _ => {
                        let json_str = serde_json::to_string(value).map_err(|e| {
                            FrameworkError::WorkflowError(format!("JSON序列化失败: {}", e))
                        })?;
                        Ok(DataValue::from_string(json_str))
                    }
                }
            }
        }
    }

    #[allow(dead_code)]
    async fn get_var_node_reads_declared_variable_default() {
        let json = r#"
        {
            "version": "1.0",
            "metadata": {
                "name": "get_var_default",
                "created": "2024-01-01T00:00:00Z",
                "modified": "2024-01-01T00:00:00Z"
            },
            "variables": [
                {"name": "label", "data_type": "String", "default_value": "hello"}
            ],
            "nodes": [
                {
                    "id": "start_1",
                    "node_type": "StartNode",
                    "pins": [{"name": "Out", "kind": "ExecOutput"}]
                },
                {
                    "id": "get_label",
                    "node_type": "GetVarNode",
                    "pins": [{"name": "Value", "kind": "DataOutput", "data_type": "Any"}],
                    "properties": {"variable_name": "label"}
                },
                {
                    "id": "end_1",
                    "node_type": "EndNode",
                    "pins": [
                        {"name": "In", "kind": "ExecInput"},
                        {"name": "result", "kind": "DataInput", "data_type": "Any"}
                    ]
                }
            ],
            "connections": [
                {
                    "source_node": "start_1",
                    "source_pin": "Out",
                    "target_node": "end_1",
                    "target_pin": "In",
                    "connection_type": "Exec"
                },
                {
                    "source_node": "get_label",
                    "source_pin": "Value",
                    "target_node": "end_1",
                    "target_pin": "result",
                    "connection_type": "Data"
                }
            ]
        }
        "#;

        use crate::orchestration::Orchestrator;
        use crate::workflow::execution::ExecutionContext;
        use std::collections::HashMap;

        let orchestrator = Orchestrator::builder().build();
        let ctx = orchestrator.create_context();
        let loaded = BlueprintLoader::new()
            .load_from_json_str(json, &ctx)
            .unwrap();
        let mut exec_ctx = ExecutionContext::from_context(ctx);
        loaded
            .compiled
            .initialize_defaults(&mut exec_ctx)
            .await
            .unwrap();
        let outputs = loaded
            .compiled
            .executor()
            .execute_with_params(&mut exec_ctx, HashMap::new())
            .await
            .unwrap();

        assert_eq!(
            outputs.get("result").and_then(|value| value.as_str()),
            Some("hello")
        );
    }

    #[allow(dead_code)]
    fn get_var_node_rejects_undeclared_variable() {
        let json = r#"
        {
            "version": "1.0",
            "metadata": {
                "name": "get_var_reject",
                "created": "2024-01-01T00:00:00Z",
                "modified": "2024-01-01T00:00:00Z"
            },
            "nodes": [
                {
                    "id": "start_1",
                    "node_type": "StartNode",
                    "pins": [{"name": "Out", "kind": "ExecOutput"}]
                },
                {
                    "id": "get_missing",
                    "node_type": "GetVarNode",
                    "pins": [{"name": "Value", "kind": "DataOutput", "data_type": "Any"}],
                    "properties": {"variable_name": "missing"}
                },
                {
                    "id": "end_1",
                    "node_type": "EndNode",
                    "pins": [{"name": "In", "kind": "ExecInput"}]
                }
            ],
            "connections": [
                {
                    "source_node": "start_1",
                    "source_pin": "Out",
                    "target_node": "end_1",
                    "target_pin": "In",
                    "connection_type": "Exec"
                }
            ]
        }
        "#;

        use crate::orchestration::Orchestrator;

        let orchestrator = Orchestrator::builder().build();
        let ctx = orchestrator.create_context();
        let error = BlueprintLoader::new()
            .load_from_json_str(json, &ctx)
            .expect_err("undeclared GetVarNode reference should fail");
        assert!(error.to_string().contains("GetVarNode"));
    }
}

impl Default for BlueprintLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// 蓝图保存器
pub struct BlueprintSaver;

impl BlueprintSaver {
    /// 将蓝图保存为 JSON 格式
    pub fn save_to_json(_compiled: &CompiledBlueprint) -> Result<BlueprintJson> {
        // Future: 从 CompiledBlueprint 提取信息并转换为 JSON
        // 目前 CompiledBlueprint 没有提供足够的反射信息
        // 需要扩展 API

        Err(FrameworkError::WorkflowError(
            "蓝图保存功能尚未实现".to_string(),
        ))
    }

    /// 保存到文件
    pub fn save_to_file(compiled: &CompiledBlueprint, path: impl AsRef<Path>) -> Result<()> {
        let blueprint_json = Self::save_to_json(compiled)?;

        let json_str = blueprint_json
            .to_json_string()
            .map_err(|e| FrameworkError::WorkflowError(format!("JSON 序列化失败: {}", e)))?;

        std::fs::write(path.as_ref(), json_str)
            .map_err(|e| FrameworkError::WorkflowError(format!("写入文件失败: {}", e)))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_empty_blueprint() {
        let json = r#"
        {
            "version": "1.0",
            "metadata": {
                "name": "测试蓝图",
                "created": "2024-01-01T00:00:00Z",
                "modified": "2024-01-01T00:00:00Z"
            },
            "nodes": [
                {
                    "id": "start_1",
                    "node_type": "StartNode",
                    "position": {"x": 0, "y": 0},
                    "pins": [
                        {"name": "Out", "kind": "ExecOutput"}
                    ]
                },
                {
                    "id": "end_1",
                    "node_type": "EndNode",
                    "position": {"x": 200, "y": 0},
                    "pins": [
                        {"name": "In", "kind": "ExecInput"}
                    ]
                }
            ],
            "connections": [
                {
                    "source_node": "start_1",
                    "source_pin": "Out",
                    "target_node": "end_1",
                    "target_pin": "In"
                }
            ]
        }
        "#;

        use crate::orchestration::Orchestrator;
        let orchestrator = Orchestrator::builder().build();
        let ctx = orchestrator.create_context();
        let loader = BlueprintLoader::new();
        let result = loader.load_from_json_str(json, &ctx);

        match result {
            Ok(_) => tracing::debug!("空蓝图加载成功"),
            Err(e) => tracing::debug!("空蓝图加载失败: {}", e),
        }
    }
}
