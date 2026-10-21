//! Object construction carrier used by structured argument compilation.
use crate::workflow::{
    core::{DataValue, NodeOutput, Pin},
    execution::ExecutionContext,
    nodes::traits::{BlueprintNode, NodeType},
};
use crate::{
    error::{FrameworkError, Result},
    register_node,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
#[register_node(
    node_type = "Pure", version = "1.0.0", category = "Container",
    display_name = "JSON 常量 {Json}", description = "从 JSON 文字恢复常量，保留对象、数组和 null 类型",
    permissions = 0, data_in = ["Json:String@JSON 常量文字"], data_out = ["Value:Any@JSON 值"]
)]
pub struct JsonLiteralNode;

impl BlueprintNode for JsonLiteralNode {
    fn name(&self) -> &str {
        "JsonLiteral"
    }
    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }
    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Json", "String"),
            Pin::data_out("Value", "Any"),
        ]
    }
    fn execute_node<'a>(
        &'a self,
        _ctx: &'a mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<NodeOutput>> + Send + 'a>> {
        Box::pin(async move { Ok(NodeOutput::Data(self.evaluate(inputs)?)) })
    }
}

impl JsonLiteralNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let text = inputs
            .get("Json")
            .and_then(DataValue::as_str)
            .ok_or_else(|| {
                FrameworkError::InvalidOperation("JsonLiteral requires JSON text".into())
            })?;
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| FrameworkError::InvalidOperation(e.to_string()))?;
        let data_type = match &value {
            serde_json::Value::Null => "Null",
            serde_json::Value::Object(_) => "Object",
            serde_json::Value::Array(_) => "Array<Any>",
            serde_json::Value::String(_) => "String",
            serde_json::Value::Bool(_) => "bool",
            _ => "num",
        };
        Ok(HashMap::from([(
            "Value".into(),
            DataValue::new(data_type, value),
        )]))
    }
}

#[derive(Debug, Clone, Default)]
#[register_node(
    node_type = "Pure", version = "1.0.0", category = "Container",
    display_name = "用键 {Keys} 和值 {Values} 构造对象", description = "按 Keys 与 Values 一一对应构造对象",
    permissions = 0,
    data_in = ["Keys:Array<String>@对象字段名", "Values:Array<Any>@对象字段值"],
    data_out = ["Object:Object@构造的对象"]
)]
pub struct MakeObjectNode;

impl MakeObjectNode {
    pub fn evaluate(
        &self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        let invalid = || {
            FrameworkError::InvalidOperation(
                "MakeObject requires equally sized key/value arrays with unique string keys".into(),
            )
        };
        let keys = inputs
            .get("Keys")
            .and_then(DataValue::as_array)
            .ok_or_else(invalid)?;
        let values = inputs
            .get("Values")
            .and_then(DataValue::as_array)
            .ok_or_else(invalid)?;
        if keys.len() != values.len() {
            return Err(invalid());
        }
        let mut object = serde_json::Map::new();
        for (key, value) in keys.iter().zip(values) {
            let key = key.as_str().ok_or_else(invalid)?;
            if object.insert(key.into(), value.clone()).is_some() {
                return Err(invalid());
            }
        }
        Ok(HashMap::from([(
            "Object".into(),
            DataValue::new("Object", serde_json::Value::Object(object)),
        )]))
    }
}

impl BlueprintNode for MakeObjectNode {
    fn name(&self) -> &str {
        "MakeObject"
    }
    fn node_type(&self) -> NodeType {
        NodeType::Pure
    }
    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("Keys", "Array<String>"),
            Pin::data_in("Values", "Array<Any>"),
            Pin::data_out("Object", "Object"),
        ]
    }
    fn execute_node<'a>(
        &'a self,
        _ctx: &'a mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<NodeOutput>> + Send + 'a>> {
        Box::pin(async move { Ok(NodeOutput::Data(self.evaluate(inputs)?)) })
    }
}
