//!
//! 类似 UE 的结构体引脚拆分功能，允许将聚合类型拆分为字段访问

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

use crate::error::{FrameworkError, Result};
use crate::workflow::core::DataValue;

/// 引脚路径 - 支持嵌套字段访问
///
/// 示例：
/// - `rect` → 基础引脚
/// - `rect.top_left` → 拆分一层  
/// - `rect.top_left.x` → 拆分两层
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PinPath {
    /// 基础引脚名（如 "rect"）
    pub base_name: String,

    /// 字段路径（如 ["top_left", "x"]）
    pub field_path: Vec<String>,
}

impl PinPath {
    /// 创建基础引脚路径
    pub fn base(name: impl Into<String>) -> Self {
        Self {
            base_name: name.into(),
            field_path: Vec::new(),
        }
    }

    /// 创建子引脚路径
    pub fn sub(base: impl Into<String>, field_path: Vec<String>) -> Self {
        Self {
            base_name: base.into(),
            field_path,
        }
    }

    /// 从字符串解析（如 "rect.top_left.x"）
    pub fn from_str(path: &str) -> Self {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.is_empty() {
            return Self::base("");
        }

        Self {
            base_name: parts[0].to_string(),
            field_path: parts[1..].iter().map(|s| s.to_string()).collect(),
        }
    }

    /// 转为字符串
    pub fn to_string(&self) -> String {
        if self.field_path.is_empty() {
            self.base_name.clone()
        } else {
            format!("{}.{}", self.base_name, self.field_path.join("."))
        }
    }

    /// 是否为子引脚
    pub fn is_sub_pin(&self) -> bool {
        !self.field_path.is_empty()
    }
}

/// 字段映射 - 记录字段名、类型和输出引脚
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldMapping {
    /// 字段名
    pub field_name: String,

    /// 字段类型
    pub field_type: String,

    /// 输出引脚名（可能与字段名不同）
    pub output_pin: String,

    /// 是否可选
    #[serde(default)]
    pub optional: bool,
}

impl FieldMapping {
    pub fn new(
        field_name: impl Into<String>,
        field_type: impl Into<String>,
        output_pin: impl Into<String>,
    ) -> Self {
        Self {
            field_name: field_name.into(),
            field_type: field_type.into(),
            output_pin: output_pin.into(),
            optional: false,
        }
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

/// 字段拆解器 - 将聚合类型拆分为字段
///
/// 功能：从结构体 JSON 中提取字段值
#[derive(Debug, Clone)]
pub struct FieldDisassembler {
    /// 输入类型
    pub input_type: String,

    /// 字段映射
    pub field_mappings: Vec<FieldMapping>,
}

impl FieldDisassembler {
    pub fn new(input_type: impl Into<String>) -> Self {
        Self {
            input_type: input_type.into(),
            field_mappings: Vec::new(),
        }
    }

    pub fn add_field(mut self, mapping: FieldMapping) -> Self {
        self.field_mappings.push(mapping);
        self
    }

    /// 执行拆解
    pub fn evaluate(&self, input: DataValue) -> Result<HashMap<String, DataValue>> {
        // 验证输入类型
        if input.type_name != self.input_type {
            return Err(FrameworkError::WorkflowError(format!(
                "Disassembler type mismatch: expected {}, got {}",
                self.input_type, input.type_name
            )));
        }

        // 从 JSON 中提取字段
        let obj = input.value.as_object().ok_or_else(|| {
            FrameworkError::WorkflowError(format!("Expected object type for {}", self.input_type))
        })?;

        let mut outputs = HashMap::new();

        for mapping in &self.field_mappings {
            if let Some(field_value) = obj.get(&mapping.field_name) {
                outputs.insert(
                    mapping.output_pin.clone(),
                    DataValue::new(&mapping.field_type, field_value.clone()),
                );
            } else if !mapping.optional {
                return Err(FrameworkError::WorkflowError(format!(
                    "Required field '{}' not found in {}",
                    mapping.field_name, self.input_type
                )));
            }
        }

        Ok(outputs)
    }
}

/// 字段组装器 - 将字段合并为聚合类型
///
/// 功能：从多个字段值构建结构体 JSON
#[derive(Debug, Clone)]
pub struct FieldAssembler {
    /// 输出类型
    pub output_type: String,

    /// 字段映射
    pub field_mappings: Vec<FieldMapping>,

    /// 默认值（用于未连接的可选字段）
    pub default_values: HashMap<String, JsonValue>,
}

impl FieldAssembler {
    pub fn new(output_type: impl Into<String>) -> Self {
        Self {
            output_type: output_type.into(),
            field_mappings: Vec::new(),
            default_values: HashMap::new(),
        }
    }

    pub fn add_field(mut self, mapping: FieldMapping) -> Self {
        self.field_mappings.push(mapping);
        self
    }

    pub fn set_default(mut self, field_name: impl Into<String>, value: JsonValue) -> Self {
        self.default_values.insert(field_name.into(), value);
        self
    }

    /// 执行组装
    pub fn evaluate(&self, inputs: HashMap<String, DataValue>) -> Result<DataValue> {
        let mut obj = serde_json::Map::new();

        for mapping in &self.field_mappings {
            // 查找输入值
            if let Some(input) = inputs.get(&mapping.field_name) {
                // 验证类型
                if input.type_name != mapping.field_type {
                    return Err(FrameworkError::WorkflowError(format!(
                        "Field '{}' type mismatch: expected {}, got {}",
                        mapping.field_name, mapping.field_type, input.type_name
                    )));
                }
                obj.insert(mapping.field_name.clone(), input.value.clone());
            } else if let Some(default) = self.default_values.get(&mapping.field_name) {
                // 使用默认值
                obj.insert(mapping.field_name.clone(), default.clone());
            } else if !mapping.optional {
                return Err(FrameworkError::WorkflowError(format!(
                    "Required field '{}' is missing for {}",
                    mapping.field_name, self.output_type
                )));
            }
        }

        Ok(DataValue::new(&self.output_type, JsonValue::Object(obj)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_pin_path_parsing() {
        let path = PinPath::from_str("rect");
        assert_eq!(path.base_name, "rect");
        assert_eq!(path.field_path, Vec::<String>::new());
        assert!(!path.is_sub_pin());

        let path = PinPath::from_str("rect.top_left");
        assert_eq!(path.base_name, "rect");
        assert_eq!(path.field_path, vec!["top_left"]);
        assert!(path.is_sub_pin());

        let path = PinPath::from_str("rect.top_left.x");
        assert_eq!(path.base_name, "rect");
        assert_eq!(path.field_path, vec!["top_left", "x"]);
        assert!(path.is_sub_pin());
    }

    #[test]
    fn test_pin_path_to_string() {
        let path = PinPath::base("rect");
        assert_eq!(path.to_string(), "rect");

        let path = PinPath::sub("rect", vec!["top_left".to_string()]);
        assert_eq!(path.to_string(), "rect.top_left");

        let path = PinPath::sub("rect", vec!["top_left".to_string(), "x".to_string()]);
        assert_eq!(path.to_string(), "rect.top_left.x");
    }

    #[test]
    fn test_disassembler() {
        // 构建 Rectangle 数据
        let rect_value = DataValue::new(
            "Rectangle",
            json!({
                "top_left": { "x": 0.0, "y": 0.0 },
                "bottom_right": { "x": 100.0, "y": 100.0 }
            }),
        );

        // 创建拆解器
        let disassembler = FieldDisassembler::new("Rectangle")
            .add_field(FieldMapping::new("top_left", "Point2D", "top_left"))
            .add_field(FieldMapping::new("bottom_right", "Point2D", "bottom_right"));

        // 执行拆解
        let result = disassembler.evaluate(rect_value).unwrap();

        assert_eq!(result.len(), 2);

        let top_left = result.get("top_left").unwrap();
        assert_eq!(top_left.type_name, "Point2D");
        assert_eq!(top_left.value, json!({ "x": 0.0, "y": 0.0 }));
    }

    #[test]
    fn test_assembler() {
        // 构建输入字段
        let mut inputs = HashMap::new();
        inputs.insert(
            "top_left".to_string(),
            DataValue::new("Point2D", json!({ "x": 0.0, "y": 0.0 })),
        );
        inputs.insert(
            "bottom_right".to_string(),
            DataValue::new("Point2D", json!({ "x": 100.0, "y": 100.0 })),
        );

        // 创建组装器
        let assembler = FieldAssembler::new("Rectangle")
            .add_field(FieldMapping::new("top_left", "Point2D", "top_left"))
            .add_field(FieldMapping::new("bottom_right", "Point2D", "bottom_right"));

        // 执行组装
        let result = assembler.evaluate(inputs).unwrap();

        assert_eq!(result.type_name, "Rectangle");
        assert_eq!(
            result.value,
            json!({
                "top_left": { "x": 0.0, "y": 0.0 },
                "bottom_right": { "x": 100.0, "y": 100.0 }
            })
        );
    }
}
