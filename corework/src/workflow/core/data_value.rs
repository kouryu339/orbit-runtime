//! DataValue - 携带类型信息的数据值
//!
//! 从 blueprint.rs 提取的 DataValue 相关类型

use super::pin::PinContainerType;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Data value carried on data pins
/// 支持：i64, f64, String, bool, Array, Object, Null 等类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataValue {
    pub type_name: String, // 完整类型名，如 "Vec<String>", "Object", "Null"
    pub value: JsonValue,  // 实际数据
    /// 容器元素类型（用于数组/Option）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_type: Option<String>,
    /// 容器类型
    #[serde(default)]
    pub container: PinContainerType,
}

impl DataValue {
    /// 创建新的 DataValue
    pub fn new(type_name: impl Into<String>, value: JsonValue) -> Self {
        Self {
            type_name: type_name.into(),
            value,
            element_type: None,
            container: PinContainerType::None,
        }
    }

    // ============ 工厂函数 (Factory Methods) ============

    /// 创建整数值
    pub fn from_i64(value: i64) -> Self {
        Self::new("i64", JsonValue::Number(value.into()))
    }

    /// 创建浮点数值
    pub fn from_f64(value: f64) -> Self {
        Self::new(
            "f64",
            JsonValue::Number(serde_json::Number::from_f64(value).unwrap()),
        )
    }

    /// 创建字符串值
    pub fn from_string(value: impl Into<String>) -> Self {
        Self::new("String", JsonValue::String(value.into()))
    }

    /// 创建路径值（Path 是 String 的别名，底层存储相同）
    pub fn from_path(value: impl Into<String>) -> Self {
        Self::new("Path", JsonValue::String(value.into()))
    }

    /// 创建日期值（Date 是 String 的别名，格式 "YYYY-MM-DD"）
    pub fn from_date(value: impl Into<String>) -> Self {
        Self::new("Date", JsonValue::String(value.into()))
    }

    /// 创建时间值（Time 是 String 的别名，格式 "HH:MM" 或 "HH:MM:SS"）
    pub fn from_time(value: impl Into<String>) -> Self {
        Self::new("Time", JsonValue::String(value.into()))
    }

    /// 创建布尔值
    pub fn from_bool(value: bool) -> Self {
        Self::new("bool", JsonValue::Bool(value))
    }

    /// 创建 Null 值
    pub fn null() -> Self {
        Self::new("Null", JsonValue::Null)
    }

    /// 创建数组值
    pub fn from_array<T: Serialize>(items: Vec<T>, element_type: impl Into<String>) -> Self {
        let elem_type = element_type.into();
        Self {
            type_name: format!("Vec<{}>", elem_type),
            value: serde_json::to_value(items).unwrap_or(JsonValue::Array(vec![])),
            element_type: Some(elem_type),
            container: PinContainerType::Array,
        }
    }

    /// 创建对象值（从 HashMap<String, DataValue>）
    pub fn from_object(data: HashMap<String, DataValue>) -> Self {
        let mut obj = serde_json::Map::new();
        for (key, val) in data {
            obj.insert(key, val.value.clone());
        }
        Self {
            type_name: "Object".to_string(),
            value: JsonValue::Object(obj),
            element_type: None,
            container: PinContainerType::None,
        }
    }

    /// 创建对象值（从 JSON 对象）
    pub fn from_json_object(obj: JsonValue) -> crate::error::Result<Self> {
        if !obj.is_object() {
            return Err(crate::error::FrameworkError::SystemError(
                "Expected object value".to_string(),
            ));
        }
        Ok(Self {
            type_name: "Object".to_string(),
            value: obj,
            element_type: None,
            container: PinContainerType::None,
        })
    }

    // ============ 访问方法 (Accessor Methods) ============

    /// 以 i64 形式访问，失败返回 None
    pub fn as_i64(&self) -> Option<i64> {
        self.value.as_i64()
    }

    /// 以 f64 形式访问，失败返回 None
    pub fn as_f64(&self) -> Option<f64> {
        self.value.as_f64()
    }

    /// 以字符串形式访问，失败返回 None
    pub fn as_str(&self) -> Option<&str> {
        self.value.as_str()
    }

    /// 以路径字符串形式访问（Path 与 String 底层相同，均返回 &str）
    pub fn as_path(&self) -> Option<&str> {
        self.value.as_str()
    }

    /// 检查是否为路径类型
    pub fn is_path(&self) -> bool {
        self.type_name == "Path"
    }

    /// 以布尔值形式访问，失败返回 None
    pub fn as_bool(&self) -> Option<bool> {
        self.value.as_bool()
    }

    /// 检查是否为数组
    pub fn is_array(&self) -> bool {
        self.container == PinContainerType::Array || self.value.is_array()
    }

    /// 以数组形式访问，失败返回 None
    pub fn as_array(&self) -> Option<&Vec<JsonValue>> {
        self.value.as_array()
    }

    /// 获取可变的数组引用
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<JsonValue>> {
        self.value.as_array_mut()
    }

    /// 检查是否为对象
    pub fn is_object(&self) -> bool {
        self.value.is_object() || self.type_name == "Object"
    }

    /// 以对象形式访问，失败返回 None
    pub fn as_object(&self) -> Option<&serde_json::Map<String, JsonValue>> {
        self.value.as_object()
    }

    /// 获取可变的对象引用
    pub fn as_object_mut(&mut self) -> Option<&mut serde_json::Map<String, JsonValue>> {
        self.value.as_object_mut()
    }

    /// 检查是否为 Null
    pub fn is_null(&self) -> bool {
        self.value.is_null()
    }

    // ============ 对象字段操作 (Object Field Methods) ============

    /// 获取对象字段值（单级）
    pub fn get_field(&self, key: &str) -> Option<DataValue> {
        self.as_object()
            .and_then(|obj| obj.get(key))
            .map(|val| DataValue::new("Any", val.clone()))
    }

    /// 获取对象字段值（支持路径访问，如 "field.subfield.value"）
    pub fn get_field_path(&self, path: &str) -> Option<DataValue> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.is_empty() {
            return None;
        }

        // 从当前值开始，逐级访问
        let mut current_value = self.value.clone();

        for part in parts {
            if let Some(obj) = current_value.as_object() {
                if let Some(next_value) = obj.get(part) {
                    current_value = next_value.clone();
                } else {
                    return None; // 路径不存在
                }
            } else {
                return None; // 不是对象，无法继续访问
            }
        }

        // 返回找到的值，类型设为 Any（运行时动态类型）
        Some(DataValue::new("Any", current_value))
    }

    /// 设置对象字段值（单级）
    pub fn set_field(
        &mut self,
        key: impl Into<String>,
        value: DataValue,
    ) -> crate::error::Result<()> {
        if !self.is_object() {
            return Err(crate::error::FrameworkError::SystemError(format!(
                "Expected object type, got {}",
                self.type_name
            )));
        }
        self.as_object_mut()
            .unwrap()
            .insert(key.into(), value.value.clone());
        Ok(())
    }

    /// 设置对象字段值（支持路径访问，如 "field.subfield.value"）
    pub fn set_field_path(&mut self, path: &str, value: DataValue) -> crate::error::Result<()> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.is_empty() {
            return Err(crate::error::FrameworkError::SystemError(
                "Empty field path".to_string(),
            ));
        }

        // 递归访问到倒数第二级
        let mut current = &mut self.value;

        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                // 最后一级，直接设置
                if let Some(obj) = current.as_object_mut() {
                    obj.insert(part.to_string(), value.value.clone());
                    return Ok(());
                } else {
                    return Err(crate::error::FrameworkError::SystemError(format!(
                        "Path parent '{}' is not an object",
                        parts[..i].join(".")
                    )));
                }
            } else if current.is_object() {
                let obj = current.as_object_mut().unwrap();
                if !obj.contains_key(*part) {
                    // 自动创建中间对象
                    obj.insert(part.to_string(), JsonValue::Object(serde_json::Map::new()));
                }
                current = obj.get_mut(*part).unwrap();
            } else {
                return Err(crate::error::FrameworkError::SystemError(format!(
                    "Path segment '{}' is not an object",
                    parts[..=i].join(".")
                )));
            }
        }

        Ok(())
    }

    /// 获取对象所有的键
    pub fn object_keys(&self) -> Option<Vec<String>> {
        self.as_object().map(|obj| obj.keys().cloned().collect())
    }

    /// 获取对象所有的值
    pub fn object_values(&self) -> Option<Vec<JsonValue>> {
        self.as_object().map(|obj| obj.values().cloned().collect())
    }

    // ============ 数组操作 (Array Methods) ============

    /// 获取数组长度
    pub fn array_len(&self) -> Option<usize> {
        self.as_array().map(|arr| arr.len())
    }

    /// 获取数组元素
    pub fn get_array_element(&self, index: usize) -> Option<DataValue> {
        self.as_array().and_then(|arr| arr.get(index)).map(|val| {
            DataValue::new(
                self.element_type
                    .clone()
                    .unwrap_or_else(|| "Any".to_string()),
                val.clone(),
            )
        })
    }

    /// 设置数组元素
    pub fn set_array_element(
        &mut self,
        index: usize,
        value: DataValue,
    ) -> crate::error::Result<()> {
        if !self.is_array() {
            return Err(crate::error::FrameworkError::SystemError(format!(
                "Expected array type, got {}",
                self.type_name
            )));
        }
        if let Some(arr) = self.as_array_mut() {
            if index < arr.len() {
                arr[index] = value.value.clone();
                Ok(())
            } else {
                Err(crate::error::FrameworkError::SystemError(format!(
                    "Array index out of bounds: {} >= {}",
                    index,
                    arr.len()
                )))
            }
        } else {
            Err(crate::error::FrameworkError::SystemError(
                "Failed to get mutable array reference".to_string(),
            ))
        }
    }

    /// 向数组追加元素
    pub fn push_array_element(&mut self, value: DataValue) -> crate::error::Result<()> {
        if !self.is_array() {
            return Err(crate::error::FrameworkError::SystemError(format!(
                "Expected array type, got {}",
                self.type_name
            )));
        }
        if let Some(arr) = self.as_array_mut() {
            arr.push(value.value.clone());
            Ok(())
        } else {
            Err(crate::error::FrameworkError::SystemError(
                "Failed to get mutable array reference".to_string(),
            ))
        }
    }

    // ============ 提取和转换 (Extraction & Conversion) ============

    /// 提取数组为 Vec<T>
    pub fn extract_array<T: serde::de::DeserializeOwned>(&self) -> crate::error::Result<Vec<T>> {
        if !self.is_array() {
            return Err(crate::error::FrameworkError::SystemError(format!(
                "Expected array type, got {}",
                self.type_name
            )));
        }
        serde_json::from_value(self.value.clone())
            .map_err(crate::error::FrameworkError::SerializationError)
    }

    /// 提取对象为 HashMap<String, T>
    pub fn extract_object<T: serde::de::DeserializeOwned>(
        &self,
    ) -> crate::error::Result<HashMap<String, T>> {
        if !self.is_object() {
            return Err(crate::error::FrameworkError::SystemError(format!(
                "Expected object type, got {}",
                self.type_name
            )));
        }
        serde_json::from_value(self.value.clone())
            .map_err(crate::error::FrameworkError::SerializationError)
    }

    /// 类型转换：尝试将值转换为 i64
    pub fn to_i64(&self) -> crate::error::Result<i64> {
        match self.type_name.as_str() {
            "i64" => self.as_i64().ok_or_else(|| {
                crate::error::FrameworkError::SystemError("Failed to parse i64".to_string())
            }),
            "f64" => self.as_f64().map(|v| v as i64).ok_or_else(|| {
                crate::error::FrameworkError::SystemError("Failed to parse f64".to_string())
            }),
            "String" => self
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .ok_or_else(|| {
                    crate::error::FrameworkError::SystemError(format!(
                        "Cannot convert '{}' to i64",
                        self.as_str().unwrap_or("null")
                    ))
                }),
            "bool" => Ok(if self.as_bool().unwrap_or(false) {
                1
            } else {
                0
            }),
            _ => Err(crate::error::FrameworkError::SystemError(format!(
                "Cannot convert {} to i64",
                self.type_name
            ))),
        }
    }

    /// 类型转换：尝试将值转换为 f64
    pub fn to_f64(&self) -> crate::error::Result<f64> {
        match self.type_name.as_str() {
            "f64" => self.as_f64().ok_or_else(|| {
                crate::error::FrameworkError::SystemError("Failed to parse f64".to_string())
            }),
            "i64" => self.as_i64().map(|v| v as f64).ok_or_else(|| {
                crate::error::FrameworkError::SystemError("Failed to parse i64".to_string())
            }),
            "String" => self
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .ok_or_else(|| {
                    crate::error::FrameworkError::SystemError(format!(
                        "Cannot convert '{}' to f64",
                        self.as_str().unwrap_or("null")
                    ))
                }),
            "bool" => Ok(if self.as_bool().unwrap_or(false) {
                1.0
            } else {
                0.0
            }),
            _ => Err(crate::error::FrameworkError::SystemError(format!(
                "Cannot convert {} to f64",
                self.type_name
            ))),
        }
    }

    /// 类型转换：转换为字符串
    pub fn to_string_value(&self) -> String {
        match self.type_name.as_str() {
            "String" | "Path" | "Date" | "Time" => self.as_str().unwrap_or("").to_string(),
            "i64" => self.as_i64().map(|v| v.to_string()).unwrap_or_default(),
            "f64" => self.as_f64().map(|v| v.to_string()).unwrap_or_default(),
            "bool" => self.as_bool().map(|v| v.to_string()).unwrap_or_default(),
            "Null" => "null".to_string(),
            _ => self.value.to_string(),
        }
    }

    /// 类型转换：转换为布尔值
    pub fn to_bool(&self) -> bool {
        match self.type_name.as_str() {
            "bool" => self.as_bool().unwrap_or(false),
            "i64" => self.as_i64().unwrap_or(0) != 0,
            "f64" => self.as_f64().unwrap_or(0.0) != 0.0,
            "String" => !self.as_str().unwrap_or("").is_empty(),
            "Null" => false,
            _ => !self.value.is_null(),
        }
    }

    // ============ 序列化/反序列化 (Serialization) ============

    /// 序列化为 JSON 字符串
    pub fn to_json_string(&self) -> crate::error::Result<String> {
        serde_json::to_string(self).map_err(crate::error::FrameworkError::SerializationError)
    }

    /// 从 JSON 字符串反序列化
    pub fn from_json_string(json_str: &str) -> crate::error::Result<Self> {
        serde_json::from_str(json_str).map_err(crate::error::FrameworkError::SerializationError)
    }

    /// 获取仅包含值的 JSON 表示
    pub fn value_as_json_string(&self) -> crate::error::Result<String> {
        serde_json::to_string(&self.value).map_err(crate::error::FrameworkError::SerializationError)
    }

    // ============ 工具方法 (Utility Methods) ============

    /// 获取类型名称
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// 获取值的 JSON 引用
    pub fn json_value(&self) -> &JsonValue {
        &self.value
    }

    /// 获取值的可变 JSON 引用
    pub fn json_value_mut(&mut self) -> &mut JsonValue {
        &mut self.value
    }

    /// 检查值是否为空/无效
    pub fn is_empty(&self) -> bool {
        match self.type_name.as_str() {
            "Null" => true,
            "String" | "Path" | "Date" | "Time" => {
                self.as_str().map(|s| s.is_empty()).unwrap_or(true)
            }
            "Array" | "Vec" => self.as_array().map(|a| a.is_empty()).unwrap_or(true),
            "Object" => self.as_object().map(|o| o.is_empty()).unwrap_or(true),
            _ => false,
        }
    }

    /// 创建值的深拷贝
    pub fn deep_clone(&self) -> Self {
        Self {
            type_name: self.type_name.clone(),
            value: self.value.clone(),
            element_type: self.element_type.clone(),
            container: self.container,
        }
    }
}

// ❌ 不实现 DataType trait！
// DataValue 是运行时包装器，不应该作为用户可见的类型导出
// 如果需要"任意类型"语义，应该使用 "Any" 类型名
