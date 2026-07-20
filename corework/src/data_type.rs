//! 数据类型系统
//!
//! 提供框架级别的类型注册和管理

use crate::error::Result;
use serde_json::Value as JsonValue;
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;

/// 数据类型 trait
pub trait DataType: Send + Sync {
    fn type_name() -> &'static str
    where
        Self: Sized;
    fn serialize(&self) -> Result<JsonValue>;
    fn deserialize(value: &JsonValue) -> Result<Self>
    where
        Self: Sized;
    fn description() -> &'static str
    where
        Self: Sized,
    {
        Self::type_name()
    }
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}

// 基本类型实现
impl DataType for String {
    fn type_name() -> &'static str {
        "String"
    }
    fn description() -> &'static str {
        "String"
    }
    fn serialize(&self) -> Result<JsonValue> {
        Ok(JsonValue::String(self.clone()))
    }
    fn deserialize(value: &JsonValue) -> Result<Self> {
        value
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("not a string").into())
    }
}

impl DataType for i64 {
    fn type_name() -> &'static str {
        "i64"
    }
    fn description() -> &'static str {
        "64-bit signed integer"
    }
    fn serialize(&self) -> Result<JsonValue> {
        Ok(JsonValue::Number((*self).into()))
    }
    fn deserialize(value: &JsonValue) -> Result<Self> {
        value
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("not an integer").into())
    }
}

impl DataType for f64 {
    fn type_name() -> &'static str {
        "f64"
    }
    fn description() -> &'static str {
        "64-bit floating point"
    }
    fn serialize(&self) -> Result<JsonValue> {
        serde_json::Number::from_f64(*self)
            .map(JsonValue::Number)
            .ok_or_else(|| anyhow::anyhow!("invalid float").into())
    }
    fn deserialize(value: &JsonValue) -> Result<Self> {
        value
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("not a float").into())
    }
}

impl DataType for bool {
    fn type_name() -> &'static str {
        "bool"
    }
    fn description() -> &'static str {
        "Boolean"
    }
    fn serialize(&self) -> Result<JsonValue> {
        Ok(JsonValue::Bool(*self))
    }
    fn deserialize(value: &JsonValue) -> Result<Self> {
        value
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("not a boolean").into())
    }
}

impl DataType for u64 {
    fn type_name() -> &'static str {
        "u64"
    }
    fn serialize(&self) -> Result<JsonValue> {
        Ok(JsonValue::Number((*self).into()))
    }
    fn deserialize(value: &JsonValue) -> Result<Self> {
        value
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("not a u64").into())
    }
}

struct TypeInfo {
    description: String,
    type_id: TypeId,
}

/// 数据类型注册表
pub struct DataTypeRegistry {
    types: Arc<parking_lot::RwLock<HashMap<String, TypeInfo>>>,
}

impl DataTypeRegistry {
    pub fn new() -> Self {
        let registry = Self {
            types: Arc::new(parking_lot::RwLock::new(HashMap::new())),
        };
        registry.register_basic_types();
        registry
    }

    fn register_basic_types(&self) {
        use crate::workflow::core::builtin_types::{KeyValuePair, Object};

        // 字符串和布尔
        let _ = self.register::<String>();
        let _ = self.register::<bool>();

        // 64位整数（统一使用64位，避免精度混乱）
        let _ = self.register::<i64>();

        if let Err(e) = self.register::<u64>() {
            tracing::warn!("Warning: Failed to register u64: {}", e);
        }

        // 64位浮点数
        let _ = self.register::<f64>();

        // 复合基础类型
        if let Err(e) = self.register::<KeyValuePair>() {
            tracing::warn!("Warning: Failed to register KeyValuePair: {}", e);
        }

        // ✅ Object类型 - JSON对象（这是一个具体类型，需要注册）
        if let Err(e) = self.register::<Object>() {
            tracing::warn!("Warning: Failed to register Object: {}", e);
        }

        // 📝 注意：Array 不需要注册，它只是泛型表示（Array<T>）

        // ❌ 重要：DataValue 不应注册为类型！
        // DataValue 是运行时动态值包装器，不是用户可见的静态类型
        // 节点使用 "Any" 类型名来表示"接受任意类型"的语义
        //
        // 错误示例：data_in = ["Input:DataValue"]  ❌ 不要这样
        // 正确示例：data_in = ["Input:Any"]        ✅ 使用 Any
        //
        // ⚠️ 如果 DataValue 出现在 types.json 中，说明某处错误地实现了 DataType trait！
    }

    pub fn register<T: DataType + 'static>(&self) -> Result<()> {
        let name = T::type_name().to_string();
        let description = T::description().to_string();
        let type_id = TypeId::of::<T>();

        // 调试输出
        tracing::warn!(
            "[DEBUG] Attempting to register type: '{}' (description: '{}')",
            name,
            description
        );

        if self.types.read().contains_key(&name) {
            if matches!(name.as_str(), "String" | "i64" | "f64" | "bool") {
                tracing::warn!(
                    "[DEBUG] Type '{}' already registered (basic type, skipping)",
                    name
                );
                return Ok(());
            }
            tracing::warn!(
                "[DEBUG] Type '{}' already registered, returning error",
                name
            );
            return Err(anyhow::anyhow!("Type already registered: {}", name).into());
        }

        let info = TypeInfo {
            description,
            type_id,
        };

        self.types.write().insert(name, info);
        Ok(())
    }

    pub fn is_registered(&self, type_name: &str) -> bool {
        self.types.read().contains_key(type_name)
    }

    pub fn is_type_id_registered(&self, type_id: TypeId) -> bool {
        self.types
            .read()
            .values()
            .any(|info| info.type_id == type_id)
    }

    pub fn registered_types(&self) -> Vec<String> {
        self.types.read().keys().cloned().collect()
    }

    pub fn type_description(&self, type_name: &str) -> Option<String> {
        self.types
            .read()
            .get(type_name)
            .map(|info| info.description.clone())
    }
}

impl Default for DataTypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for DataTypeRegistry {
    fn clone(&self) -> Self {
        Self {
            types: self.types.clone(),
        }
    }
}

/// 模型类型工厂 - 用于 #[buns_model] 装饰器注册
///
/// 通过 inventory 机制自动收集所有标注了 #[buns_model] 的类型
pub struct ModelTypeFactory {
    pub name: &'static str,
    pub type_id: TypeId,
}

inventory::collect!(ModelTypeFactory);

/// 获取类型名称（从注册表）
///
/// # 示例
/// ```rust
/// # use corework::prelude::type_name_of;
/// struct OcrResult;
///
/// let name = type_name_of::<OcrResult>();
/// assert!(name.is_none());
/// ```
pub fn type_name_of<T: 'static>() -> Option<&'static str> {
    let type_id = TypeId::of::<T>();
    for factory in inventory::iter::<ModelTypeFactory> {
        if factory.type_id == type_id {
            return Some(factory.name);
        }
    }
    None
}

#[macro_export]
macro_rules! impl_data_type {
    ($type:ty, $desc:expr) => {
        impl $crate::data_type::DataType for $type {
            fn type_name() -> &'static str {
                stringify!($type)
            }

            fn serialize(&self) -> $crate::error::Result<serde_json::Value> {
                Ok(serde_json::to_value(self)?)
            }

            fn deserialize(value: &serde_json::Value) -> $crate::error::Result<Self> {
                Ok(serde_json::from_value(value.clone())?)
            }

            fn description() -> &'static str {
                $desc
            }
        }
    };
}

// Type structure validation system (feature-gated)
#[cfg(feature = "type_structure")]
pub mod type_structure;
