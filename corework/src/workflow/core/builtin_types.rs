//! 内置类型定义 (Built-in Type Definitions)
//!
//! 定义 KeyValuePair 和 Object 等内置复合类型

use serde::{Deserialize, Serialize};
use serde_json::json;

/// KeyValuePair - 键值对类型（基础类型）
/// 用于构造对象，格式：{ "key": "string", "value": {...} }
/// 注：作为基础容器类型，不导出为复合类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValuePair {
    /// 键名
    pub key: String,
    /// 值数据（支持任意类型）
    pub value: serde_json::Value,
}

impl KeyValuePair {
    pub fn new(key: impl Into<String>, value: serde_json::Value) -> Self {
        Self {
            key: key.into(),
            value,
        }
    }

    /// 将 KeyValuePair 序列化为 JsonValue
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "key": self.key,
            "value": self.value
        })
    }

    /// 从 JsonValue 反序列化为 KeyValuePair
    pub fn from_json(val: &serde_json::Value) -> crate::error::Result<Self> {
        let obj = val.as_object().ok_or_else(|| {
            crate::error::FrameworkError::SystemError("KeyValuePair: Expected object".to_string())
        })?;

        let key = obj
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::error::FrameworkError::SystemError(
                    "KeyValuePair: Missing or invalid 'key' field".to_string(),
                )
            })?
            .to_string();

        let value = obj.get("value").cloned().ok_or_else(|| {
            crate::error::FrameworkError::SystemError(
                "KeyValuePair: Missing 'value' field".to_string(),
            )
        })?;

        Ok(Self { key, value })
    }
}

/// 实现 DataType trait，让 KeyValuePair 可以被注册为基础类型
impl crate::data_type::DataType for KeyValuePair {
    fn type_name() -> &'static str {
        "KeyValuePair"
    }

    fn serialize(&self) -> crate::error::Result<serde_json::Value> {
        Ok(self.to_json())
    }

    fn deserialize(value: &serde_json::Value) -> crate::error::Result<Self> {
        Self::from_json(value)
    }

    fn description() -> &'static str {
        "Key-value pair for object construction"
    }
}

/// Object类型 - JSON对象类型
/// 在运行时表示为 serde_json::Map<String, Value>
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Object;

impl crate::data_type::DataType for Object {
    fn type_name() -> &'static str {
        "Object"
    }

    fn serialize(&self) -> crate::error::Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    fn deserialize(_value: &serde_json::Value) -> crate::error::Result<Self> {
        Ok(Object)
    }

    fn description() -> &'static str {
        "JSON object type (key-value map)"
    }
}

/// 已注册的内置类型列表
///
/// 说明：
/// - "Array" 列在这里只是为了类型检查，实际使用时总是带泛型：Array<T>
/// - "Object" 是一个具体的类型，代表JSON对象（已实现DataType trait并注册）
/// - "KeyValuePair" 是Object的构造材料（已实现DataType trait并注册）
/// - "Path" 是 String 的别名，语义上表示文件/目录路径，底层存储同 String
pub const BUILTIN_TYPES: &[&str] = &[
    "i64",
    "f64",
    "String",
    "bool",
    "Null",
    "Array",        // 泛型表示，实际使用 Array<T>
    "Object",       // ✅ 具体类型，已注册
    "KeyValuePair", // ✅ 具体类型，已注册
    "Path",         // ✅ String 别名，表示文件/目录路径
    "Date",         // ✅ String 别名，ISO 8601 日期字符串 "YYYY-MM-DD"
    "Time",         // ✅ String 别名，时间字符串 "HH:MM" 或 "HH:MM:SS"
];

/// 检查类型是否已注册
pub fn is_builtin_type(type_name: &str) -> bool {
    BUILTIN_TYPES.contains(&type_name)
        || type_name.starts_with("Vec<")
        || type_name.starts_with("Array<")
}

/// Path 是 String 的别名 ── 检查两个类型名是否兼容（可互相赋值）
/// 规则：Path / Date / Time 与 String 双向兼容，其余类型须完全相同
pub fn types_compatible(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    const STRING_ALIASES: &[&str] = &["String", "Path", "Date", "Time"];
    STRING_ALIASES.contains(&a) && STRING_ALIASES.contains(&b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kvpair_creation() {
        let pair = KeyValuePair::new("name", json!("Alice"));
        assert_eq!(pair.key, "name");
        assert_eq!(pair.value, json!("Alice"));
    }

    #[test]
    fn test_kvpair_to_json() {
        let pair = KeyValuePair::new("count", json!(42));
        let json_val = pair.to_json();

        assert!(json_val.is_object());
        assert_eq!(json_val["key"].as_str(), Some("count"));
        assert_eq!(json_val["value"].as_i64(), Some(42));
    }

    #[test]
    fn test_kvpair_from_json() {
        let json_val = json!({
            "key": "status",
            "value": true
        });

        let pair = KeyValuePair::from_json(&json_val).unwrap();
        assert_eq!(pair.key, "status");
        assert_eq!(pair.value, json!(true));
    }

    #[test]
    fn test_builtin_types() {
        assert!(is_builtin_type("i64"));
        assert!(is_builtin_type("String"));
        assert!(is_builtin_type("KeyValuePair"));
        assert!(is_builtin_type("Vec<String>"));
        assert!(is_builtin_type("Array<i64>"));
    }
}
