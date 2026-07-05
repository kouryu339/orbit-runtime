//! 基础数据类型操作节点
//!
//! 提供对基础类型（int, float, string, bool）的常用操作

use super::blueprint::{BlueprintNode, DataValue, NodeOutput, Pin};
use crate::error::Result;
use crate::orchestration::Context;
use async_trait::async_trait;
use std::collections::HashMap;

// ==================== 数学运算节点 ====================

/// Add 节点 - 加法（支持 i64 和 f64）
#[derive(Debug, Clone)]
pub struct AddNode {
    name: String,
    is_float: bool,
}

impl AddNode {
    pub fn new_int(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_float: false,
        }
    }

    pub fn new_float(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_float: true,
        }
    }
}

#[async_trait]
impl BlueprintNode for AddNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        let type_name = if self.is_float { "f64" } else { "i64" };
        vec![
            Pin::data_in("a", type_name),
            Pin::data_in("b", type_name),
            Pin::data_out("result", type_name),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let mut output = HashMap::new();

        if self.is_float {
            let a = input_data.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let b = input_data.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
            output.insert("result".to_string(), DataValue::from_f64(a + b));
        } else {
            let a = input_data.get("a").and_then(|v| v.as_i64()).unwrap_or(0);
            let b = input_data.get("b").and_then(|v| v.as_i64()).unwrap_or(0);
            output.insert("result".to_string(), DataValue::from_i64(a + b));
        }

        Ok(NodeOutput::Data(output))
    }
}

/// Multiply 节点 - 乘法
#[derive(Debug, Clone)]
pub struct MultiplyNode {
    name: String,
    is_float: bool,
}

impl MultiplyNode {
    pub fn new_int(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_float: false,
        }
    }

    pub fn new_float(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_float: true,
        }
    }
}

#[async_trait]
impl BlueprintNode for MultiplyNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        let type_name = if self.is_float { "f64" } else { "i64" };
        vec![
            Pin::data_in("a", type_name),
            Pin::data_in("b", type_name),
            Pin::data_out("result", type_name),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let mut output = HashMap::new();

        if self.is_float {
            let a = input_data.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let b = input_data.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
            output.insert("result".to_string(), DataValue::from_f64(a * b));
        } else {
            let a = input_data.get("a").and_then(|v| v.as_i64()).unwrap_or(0);
            let b = input_data.get("b").and_then(|v| v.as_i64()).unwrap_or(0);
            output.insert("result".to_string(), DataValue::from_i64(a * b));
        }

        Ok(NodeOutput::Data(output))
    }
}

// ==================== 比较节点 ====================

/// Greater 节点 - 大于比较
#[derive(Debug, Clone)]
pub struct GreaterNode {
    name: String,
    is_float: bool,
}

impl GreaterNode {
    pub fn new_int(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_float: false,
        }
    }

    pub fn new_float(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_float: true,
        }
    }
}

#[async_trait]
impl BlueprintNode for GreaterNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        let type_name = if self.is_float { "f64" } else { "i64" };
        vec![
            Pin::data_in("a", type_name),
            Pin::data_in("b", type_name),
            Pin::data_out("result", "bool"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let mut output = HashMap::new();

        let result = if self.is_float {
            let a = input_data.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let b = input_data.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
            a > b
        } else {
            let a = input_data.get("a").and_then(|v| v.as_i64()).unwrap_or(0);
            let b = input_data.get("b").and_then(|v| v.as_i64()).unwrap_or(0);
            a > b
        };

        output.insert("result".to_string(), DataValue::from_bool(result));
        Ok(NodeOutput::Data(output))
    }
}

/// Equal 节点 - 相等比较（支持多种类型）
#[derive(Debug, Clone)]
pub struct EqualNode {
    name: String,
    type_name: String,
}

impl EqualNode {
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
        }
    }
}

#[async_trait]
impl BlueprintNode for EqualNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("a", &self.type_name),
            Pin::data_in("b", &self.type_name),
            Pin::data_out("result", "bool"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let mut output = HashMap::new();

        let a_val = input_data.get("a").map(|v| &v.value);
        let b_val = input_data.get("b").map(|v| &v.value);

        let result = a_val == b_val;
        output.insert("result".to_string(), DataValue::from_bool(result));

        Ok(NodeOutput::Data(output))
    }
}

// ==================== 逻辑节点 ====================

/// AND 节点 - 逻辑与
#[derive(Debug, Clone)]
pub struct AndNode {
    name: String,
}

impl AndNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl BlueprintNode for AndNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("a", "bool"),
            Pin::data_in("b", "bool"),
            Pin::data_out("result", "bool"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let a = input_data
            .get("a")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let b = input_data
            .get("b")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut output = HashMap::new();
        output.insert("result".to_string(), DataValue::from_bool(a && b));

        Ok(NodeOutput::Data(output))
    }
}

/// OR 节点 - 逻辑或
#[derive(Debug, Clone)]
pub struct OrNode {
    name: String,
}

impl OrNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl BlueprintNode for OrNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("a", "bool"),
            Pin::data_in("b", "bool"),
            Pin::data_out("result", "bool"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let a = input_data
            .get("a")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let b = input_data
            .get("b")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut output = HashMap::new();
        output.insert("result".to_string(), DataValue::from_bool(a || b));

        Ok(NodeOutput::Data(output))
    }
}

/// NOT 节点 - 逻辑非
#[derive(Debug, Clone)]
pub struct NotNode {
    name: String,
}

impl NotNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl BlueprintNode for NotNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("value", "bool"),
            Pin::data_out("result", "bool"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let value = input_data
            .get("value")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut output = HashMap::new();
        output.insert("result".to_string(), DataValue::from_bool(!value));

        Ok(NodeOutput::Data(output))
    }
}

// ==================== 字符串节点 ====================

/// Concat 节点 - 字符串连接
#[derive(Debug, Clone)]
pub struct ConcatNode {
    name: String,
    input_count: usize,
}

impl ConcatNode {
    pub fn new(name: impl Into<String>, input_count: usize) -> Self {
        Self {
            name: name.into(),
            input_count: input_count.max(2),
        }
    }
}

#[async_trait]
impl BlueprintNode for ConcatNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        let mut pins = Vec::new();
        for i in 0..self.input_count {
            pins.push(Pin::data_in(format!("string_{}", i), "String"));
        }
        pins.push(Pin::data_out("result", "String"));
        pins
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let mut result = String::new();

        for i in 0..self.input_count {
            let key = format!("string_{}", i);
            if let Some(s) = input_data.get(&key).and_then(|v| v.as_str()) {
                result.push_str(s);
            }
        }

        let mut output = HashMap::new();
        output.insert("result".to_string(), DataValue::from_string(result));

        Ok(NodeOutput::Data(output))
    }
}

/// Format 节点 - 格式化字符串
#[derive(Debug, Clone)]
pub struct FormatStringNode {
    name: String,
    format_template: String,
}

impl FormatStringNode {
    pub fn new(name: impl Into<String>, format_template: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            format_template: format_template.into(),
        }
    }
}

#[async_trait]
impl BlueprintNode for FormatStringNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("value", "String"),
            Pin::data_out("result", "String"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let value = input_data
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let result = self.format_template.replace("{}", value);

        let mut output = HashMap::new();
        output.insert("result".to_string(), DataValue::from_string(result));

        Ok(NodeOutput::Data(output))
    }
}

// ==================== 类型转换节点 ====================

/// ToInt 节点 - 转换为整数
#[derive(Debug, Clone)]
pub struct ToIntNode {
    name: String,
}

impl ToIntNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl BlueprintNode for ToIntNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("value", "String"),
            Pin::data_out("result", "i64"),
            Pin::data_out("success", "bool"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let value = input_data
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("0");

        let mut output = HashMap::new();

        match value.parse::<i64>() {
            Ok(num) => {
                output.insert("result".to_string(), DataValue::from_i64(num));
                output.insert("success".to_string(), DataValue::from_bool(true));
            }
            Err(_) => {
                output.insert("result".to_string(), DataValue::from_i64(0));
                output.insert("success".to_string(), DataValue::from_bool(false));
            }
        }

        Ok(NodeOutput::Data(output))
    }
}

/// ToString 节点 - 转换为字符串
#[derive(Debug, Clone)]
pub struct ToStringNode {
    name: String,
}

impl ToStringNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl BlueprintNode for ToStringNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::data_in("value", "Value"),
            Pin::data_out("result", "String"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let value = input_data.get("value");

        let result = if let Some(v) = value {
            format!("{:?}", v.value)
        } else {
            String::new()
        };

        let mut output = HashMap::new();
        output.insert("result".to_string(), DataValue::from_string(result));

        Ok(NodeOutput::Data(output))
    }
}

// ==================== 常量节点 ====================

/// MakeInt 节点 - 创建整数常量
#[derive(Debug, Clone)]
pub struct MakeIntNode {
    name: String,
    value: i64,
}

impl MakeIntNode {
    pub fn new(name: impl Into<String>, value: i64) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

#[async_trait]
impl BlueprintNode for MakeIntNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![Pin::data_out("value", "i64")]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        _input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let mut output = HashMap::new();
        output.insert("value".to_string(), DataValue::from_i64(self.value));
        Ok(NodeOutput::Data(output))
    }
}

/// MakeBool 节点 - 创建布尔常量
#[derive(Debug, Clone)]
pub struct MakeBoolNode {
    name: String,
    value: bool,
}

impl MakeBoolNode {
    pub fn new(name: impl Into<String>, value: bool) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

#[async_trait]
impl BlueprintNode for MakeBoolNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![Pin::data_out("value", "bool")]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        _input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let mut output = HashMap::new();
        output.insert("value".to_string(), DataValue::from_bool(self.value));
        Ok(NodeOutput::Data(output))
    }
}
