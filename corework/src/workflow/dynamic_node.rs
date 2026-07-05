//! 动态系统节点 - 运行时按名称查找系统

use super::blueprint::{NodeOutput, Pin};
use super::nodes::traits::{BlueprintNode, NodeType};
use crate::error::{FrameworkError, Result};
use crate::orchestration::Context;
use crate::workflow::core::{DataValue, PinCacheMapping};
use crate::workflow::execution::ExecutionContext;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

/// 动态执行 trait - 所有系统都可以通过这个 trait 进行动态调用
#[async_trait]
pub trait DynamicExecute: Send + Sync {
    /// 动态执行系统
    ///
    /// 输入输出都是 JSON Value，由系统自己负责序列化/反序列化
    async fn execute_dynamic(&self, input: HashMap<String, Value>, ctx: &Context) -> Result<Value>;
}

#[derive(Debug)]
struct WorkflowToolEnvelope {
    result: Value,
    to_ai: Option<String>,
    error_code: Option<i32>,
    is_ai_output: bool,
}

fn split_ai_output_envelope(value: Value) -> WorkflowToolEnvelope {
    let Some(obj) = value.as_object() else {
        return WorkflowToolEnvelope {
            result: value,
            to_ai: None,
            error_code: None,
            is_ai_output: false,
        };
    };

    if !(obj.contains_key("result") && obj.contains_key("to_ai") && obj.contains_key("error_code"))
    {
        return WorkflowToolEnvelope {
            result: value,
            to_ai: None,
            error_code: None,
            is_ai_output: false,
        };
    }

    let result = obj.get("result").cloned().unwrap_or(Value::Null);
    let to_ai = obj
        .get("to_ai")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let error_code = obj
        .get("error_code")
        .and_then(Value::as_i64)
        .and_then(|code| i32::try_from(code).ok());

    WorkflowToolEnvelope {
        result,
        to_ai,
        error_code,
        is_ai_output: true,
    }
}

fn extract_outputs_from_value(
    output_mappings: &[PinCacheMapping],
    output_value: Value,
) -> HashMap<String, DataValue> {
    let mut outputs = HashMap::new();

    if output_mappings.len() == 1 {
        // 单输出：直接用整个值
        let mapping = &output_mappings[0];
        outputs.insert(
            mapping.pin_name.clone(),
            DataValue::new(&mapping.type_name, output_value),
        );
    } else if let Some(obj) = output_value.as_object() {
        // 多输出：从对象中提取字段
        for mapping in output_mappings {
            let field_name = mapping.field_name.as_deref().unwrap_or(&mapping.pin_name);
            if let Some(field_value) = obj.get(field_name) {
                outputs.insert(
                    mapping.pin_name.clone(),
                    DataValue::new(&mapping.type_name, field_value.clone()),
                );
            }
        }
    }

    outputs
}

/// 动态系统节点 - 运行时按名称查找和调用系统
///
/// 与 TypedSystemNode 的区别：
/// - TypedSystemNode: 编译时泛型绑定，类型安全
/// - DynamicSystemNode: 运行时名称查找，灵活配置
#[derive(Debug)]
pub struct DynamicSystemNode {
    name: String,
    /// 系统名称（在 SystemRegistry 中注册的名称）
    system_name: String,
    /// 输入Pin映射
    input_mappings: Vec<PinCacheMapping>,
    /// 输出Pin映射
    output_mappings: Vec<PinCacheMapping>,
    /// 错误输出Pin（可选）
    error_pin: Option<String>,
}

impl DynamicSystemNode {
    pub fn builder(
        name: impl Into<String>,
        system_name: impl Into<String>,
    ) -> DynamicSystemNodeBuilder {
        DynamicSystemNodeBuilder::new(name, system_name)
    }
}

/// 动态系统节点构建器
pub struct DynamicSystemNodeBuilder {
    name: String,
    system_name: String,
    input_mappings: Vec<PinCacheMapping>,
    output_mappings: Vec<PinCacheMapping>,
    error_pin: Option<String>,
}

impl DynamicSystemNodeBuilder {
    pub fn new(name: impl Into<String>, system_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            system_name: system_name.into(),
            input_mappings: Vec::new(),
            output_mappings: Vec::new(),
            error_pin: None,
        }
    }

    /// 添加输入映射：Pin名 → Cache Key
    pub fn map_input(
        mut self,
        pin_name: impl Into<String>,
        cache_key: impl Into<String>,
        type_name: impl Into<String>,
    ) -> Self {
        self.input_mappings
            .push(PinCacheMapping::new(pin_name, cache_key, type_name));
        self
    }

    /// 添加输入映射（字段名与引脚名不一致时使用）
    pub fn map_input_with_field(
        mut self,
        pin_name: impl Into<String>,
        field_name: impl Into<String>,
        cache_key: impl Into<String>,
        type_name: impl Into<String>,
    ) -> Self {
        self.input_mappings
            .push(PinCacheMapping::new(pin_name, cache_key, type_name).with_field(field_name));
        self
    }

    /// 添加输入映射（使用字段路径访问，如 "sub_questions[0].sub_id"）
    /// 这允许从 cache_key 指向的对象中提取嵌套字段
    pub fn map_input_with_path(
        mut self,
        pin_name: impl Into<String>,
        cache_key: impl Into<String>,
        field_path: impl Into<String>,
        type_name: impl Into<String>,
    ) -> Self {
        self.input_mappings
            .push(PinCacheMapping::new(pin_name, cache_key, type_name).with_field_path(field_path));
        self
    }

    /// 添加输出映射：Pin名 → Cache Key
    pub fn map_output(
        mut self,
        pin_name: impl Into<String>,
        cache_key: impl Into<String>,
        type_name: impl Into<String>,
    ) -> Self {
        self.output_mappings
            .push(PinCacheMapping::new(pin_name, cache_key, type_name));
        self
    }

    /// 添加输出映射（字段名与引脚名不一致时使用）
    pub fn map_output_with_field(
        mut self,
        pin_name: impl Into<String>,
        field_name: impl Into<String>,
        cache_key: impl Into<String>,
        type_name: impl Into<String>,
    ) -> Self {
        self.output_mappings
            .push(PinCacheMapping::new(pin_name, cache_key, type_name).with_field(field_name));
        self
    }

    /// 添加输出映射（使用字段路径写入，如 "sub_questions[0].sub_id"）
    /// 这允许将输出值写入到 cache_key 指向的对象的嵌套字段中
    pub fn map_output_with_path(
        mut self,
        pin_name: impl Into<String>,
        cache_key: impl Into<String>,
        field_path: impl Into<String>,
        type_name: impl Into<String>,
    ) -> Self {
        self.output_mappings
            .push(PinCacheMapping::new(pin_name, cache_key, type_name).with_field_path(field_path));
        self
    }

    /// 设置错误Pin
    pub fn error_pin(mut self, pin_name: impl Into<String>) -> Self {
        self.error_pin = Some(pin_name.into());
        self
    }

    pub fn build(self) -> DynamicSystemNode {
        DynamicSystemNode {
            name: self.name,
            system_name: self.system_name,
            input_mappings: self.input_mappings,
            output_mappings: self.output_mappings,
            error_pin: self.error_pin,
        }
    }
}

impl std::fmt::Debug for DynamicSystemNodeBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicSystemNodeBuilder")
            .field("name", &self.name)
            .field("system_name", &self.system_name)
            .finish()
    }
}

#[async_trait]
impl BlueprintNode for DynamicSystemNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn node_type(&self) -> NodeType {
        NodeType::Impure
    }

    fn pins(&self) -> Vec<Pin> {
        let mut pins = vec![Pin::exec_in("In"), Pin::exec_out("Out")];

        if self.error_pin.is_some() {
            pins.push(Pin::exec_out("Error"));
        }

        for mapping in &self.input_mappings {
            pins.push(Pin::data_in(&mapping.pin_name, &mapping.type_name));
        }

        for mapping in &self.output_mappings {
            pins.push(Pin::data_out(&mapping.pin_name, &mapping.type_name));
        }

        pins
    }

    fn pin_cache_mappings(&self) -> (Vec<PinCacheMapping>, Vec<PinCacheMapping>) {
        (self.input_mappings.clone(), self.output_mappings.clone())
    }

    fn description(&self) -> Option<&str> {
        Some("Dynamic system node - executes a system by name lookup at runtime")
    }

    fn category(&self) -> Option<&str> {
        Some("Systems")
    }
}

impl DynamicSystemNode {
    /// Impure节点执行方法（由宏调用）
    pub async fn execute(
        &self,
        ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        // 使用 ExecutionContext 的 inner() 方法获取底层 Context
        let legacy_ctx = ctx.inner();

        // 1. 将 inputs (HashMap<String, DataValue>) 转换为 HashMap<String, JsonValue>
        let mut input_map = HashMap::new();
        for (pin_name, data_value) in inputs {
            // 查找对应的映射来获取字段名
            let field_name = self
                .input_mappings
                .iter()
                .find(|m| m.pin_name == pin_name)
                .and_then(|m| m.field_name.as_deref())
                .unwrap_or(&pin_name);
            input_map.insert(field_name.to_string(), data_value.value);
        }

        // 2. 通过 Context API 按名称查找动态执行器
        let dynamic_exec = legacy_ctx.get_dynamic_system(&self.system_name)?;

        // 3. 调用动态执行
        let output_value = match dynamic_exec
            .execute_dynamic(input_map.clone(), legacy_ctx)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                // 日志输出详细错误信息
                tracing::debug!(
                    "[DynamicSystemNode ERROR] 节点: {} | 系统: {}",
                    self.name,
                    self.system_name
                );
                tracing::debug!(
                    "[DynamicSystemNode ERROR] 输入内容: {}",
                    serde_json::to_string(&input_map).unwrap_or_default()
                );
                tracing::debug!("[DynamicSystemNode ERROR] 反序列化失败: {e:?}");
                if self.error_pin.is_some() {
                    return Ok(NodeOutput::ExecPin("Error".to_string()));
                }
                return Err(e);
            }
        };

        // 4. AI 工具输出是两层结构：to_ai/error_code 给 AI 与诊断，
        //    result 才是 workflow 引脚继续消费的数据。
        let envelope = split_ai_output_envelope(output_value);
        if envelope.is_ai_output {
            ctx.trace_record_ai_output(
                envelope.to_ai.clone(),
                envelope.error_code.map(i64::from),
                Some(envelope.result.clone()),
            );
            if let Some(to_ai) = envelope.to_ai.as_deref() {
                ctx.record_tool_trace(&self.system_name, to_ai.to_string(), envelope.error_code);
                tracing::debug!(
                    "[DynamicSystemNode] 节点 '{}' 系统 '{}' 返回 to_ai: {}",
                    self.name,
                    self.system_name,
                    to_ai
                );
            }

            if let Some(error_code) = envelope.error_code {
                if error_code != 0 {
                    let message = envelope
                        .to_ai
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("AIOutput returned non-zero error_code");
                    if self.error_pin.is_some() {
                        tracing::warn!(
                            "[DynamicSystemNode] 节点 '{}' 系统 '{}' 返回错误 error_code={}: {}",
                            self.name,
                            self.system_name,
                            error_code,
                            message
                        );
                        ctx.trace_fail_node(
                            &self.name,
                            format!(
                                "{} returned error_code={}: {}",
                                self.system_name, error_code, message
                            ),
                        );
                        return Ok(NodeOutput::ExecPin("Error".to_string()));
                    }
                    return Err(FrameworkError::SystemError(format!(
                        "{} 执行失败(error_code={}): {}",
                        self.system_name, error_code, message
                    )));
                }
            }
        }

        // 5. 将 result 转换为 HashMap<String, DataValue> 并返回，让 executor 统一处理
        let outputs = extract_outputs_from_value(&self.output_mappings, envelope.result);

        Ok(NodeOutput::Data(outputs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn split_ai_output_envelope_extracts_result_and_metadata() {
        let envelope = split_ai_output_envelope(json!({
            "result": { "path": "D:/a.txt", "size": 12 },
            "to_ai": "已读取文件。",
            "error_code": 0
        }));

        assert!(envelope.is_ai_output);
        assert_eq!(envelope.error_code, Some(0));
        assert_eq!(envelope.to_ai.as_deref(), Some("已读取文件。"));
        assert_eq!(envelope.result, json!({ "path": "D:/a.txt", "size": 12 }));
    }

    #[test]
    fn split_ai_output_envelope_keeps_non_ai_output_compatible() {
        let value = json!({ "path": "D:/a.txt", "size": 12 });
        let envelope = split_ai_output_envelope(value.clone());

        assert!(!envelope.is_ai_output);
        assert_eq!(envelope.result, value);
        assert_eq!(envelope.error_code, None);
        assert_eq!(envelope.to_ai, None);
    }

    #[test]
    fn extract_outputs_uses_envelope_result_for_pins() {
        let mappings = vec![
            PinCacheMapping::new("path", "node:path", "String"),
            PinCacheMapping::new("size", "node:size", "i64"),
        ];
        let outputs =
            extract_outputs_from_value(&mappings, json!({ "path": "D:/a.txt", "size": 12 }));

        assert_eq!(
            outputs.get("path").and_then(DataValue::as_str),
            Some("D:/a.txt")
        );
        assert_eq!(outputs.get("size").and_then(DataValue::as_i64), Some(12));
    }
}
