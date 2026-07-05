//! Pin 到 Cache 的映射配置
//!
//! 用于 SystemNode 等节点，将节点的 Pin 自动映射到 Cache key

use crate::cache::Cache;
use crate::error::{FrameworkError, Result};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Pin与Cache的映射配置
#[derive(Debug, Clone)]
pub struct PinCacheMapping {
    /// Pin名称
    pub pin_name: String,
    /// 结构体字段名称（可选，默认与 pin_name 一致）
    pub field_name: Option<String>,
    /// Cache键名
    pub cache_key: String,
    /// 数据类型名称
    pub type_name: String,
    /// 默认值（当 cache key 不存在时使用）
    pub default_value: Option<JsonValue>,
    /// 字段路径（支持点号访问嵌套字段，如 "sub_questions[0].sub_id"）
    /// 当此字段存在时，会从 cache_key 对应的对象中提取指定路径的字段
    pub field_path: Option<String>,
}

impl PinCacheMapping {
    /// 创建新的 Pin-Cache 映射
    pub fn new(
        pin_name: impl Into<String>,
        cache_key: impl Into<String>,
        type_name: impl Into<String>,
    ) -> Self {
        let pin_name = pin_name.into();
        Self {
            pin_name,
            field_name: None,
            cache_key: cache_key.into(),
            type_name: type_name.into(),
            default_value: None,
            field_path: None,
        }
    }

    /// 设置默认值
    pub fn with_default(mut self, default_value: JsonValue) -> Self {
        self.default_value = Some(default_value);
        self
    }

    /// 指定结构体字段名（当字段名与 pin_name 不一致时使用）
    pub fn with_field(mut self, field_name: impl Into<String>) -> Self {
        self.field_name = Some(field_name.into());
        self
    }

    /// 指定字段路径（用于访问嵌套字段，如 "sub_questions[0].sub_id"）
    /// 当设置此字段后，会从 cache_key 对应的对象中提取指定路径的字段值
    pub fn with_field_path(mut self, field_path: impl Into<String>) -> Self {
        self.field_path = Some(field_path.into());
        self
    }
}

/// 从 Cache 读取并组装输入 Map（支持 field_name 映射、字段路径访问与默认值）
pub async fn read_input_map(
    cache: &dyn Cache,
    mappings: &[PinCacheMapping],
) -> Result<HashMap<String, JsonValue>> {
    use crate::cache::CacheExt;
    let mut input_map: HashMap<String, JsonValue> = HashMap::new();

    for mapping in mappings {
        let value = if let Some(field_path) = &mapping.field_path {
            // 使用字段路径访问
            match cache
                .get_field::<JsonValue>(&mapping.cache_key, field_path)
                .await?
            {
                Some(v) => v,
                None => {
                    if let Some(default) = &mapping.default_value {
                        default.clone()
                    } else {
                        return Err(FrameworkError::SystemError(format!(
                            "Field '{}' not found in cache key '{}'",
                            field_path, mapping.cache_key
                        )));
                    }
                }
            }
        } else {
            // 直接访问 cache key
            match cache.get_raw(&mapping.cache_key).await? {
                Some(v) => v,
                None => {
                    if let Some(default) = &mapping.default_value {
                        default.clone()
                    } else {
                        return Err(FrameworkError::SystemError(format!(
                            "Input not found in cache: {}",
                            mapping.cache_key
                        )));
                    }
                }
            }
        };

        let field_name = mapping.field_name.as_deref().unwrap_or(&mapping.pin_name);
        input_map.insert(field_name.to_string(), value);
    }

    Ok(input_map)
}

/// 将输出写回 Cache（支持 field_name 映射和字段路径写入）
pub async fn write_output_map(
    cache: &dyn Cache,
    mappings: &[PinCacheMapping],
    output_value: JsonValue,
) -> Result<()> {
    use crate::cache::CacheExt;

    if mappings.is_empty() {
        return Ok(());
    }

    if mappings.len() == 1 {
        let mapping = &mappings[0];

        // 如果有字段路径，使用 set_field
        if let Some(field_path) = &mapping.field_path {
            if let Some(field_name) = mapping.field_name.as_deref() {
                if let Some(obj) = output_value.as_object() {
                    if let Some(field_value) = obj.get(field_name) {
                        cache
                            .set_field(&mapping.cache_key, field_path, field_value)
                            .await?;
                        return Ok(());
                    }
                }
            }
            cache
                .set_field(&mapping.cache_key, field_path, &output_value)
                .await?;
            return Ok(());
        }

        // 否则使用原逻辑
        if let Some(field_name) = mapping.field_name.as_deref() {
            if let Some(obj) = output_value.as_object() {
                if let Some(field_value) = obj.get(field_name) {
                    cache
                        .set_raw(&mapping.cache_key, field_value.clone(), None)
                        .await?;
                    return Ok(());
                }
            }
        }
        cache
            .set_raw(&mapping.cache_key, output_value, None)
            .await?;
        return Ok(());
    }

    let obj = output_value.as_object().ok_or_else(|| {
        FrameworkError::SystemError("Output must be an object for multi-output mapping".to_string())
    })?;

    for mapping in mappings {
        let field_name = mapping.field_name.as_deref().unwrap_or(&mapping.pin_name);
        let field_value = obj.get(field_name).ok_or_else(|| {
            FrameworkError::SystemError(format!("Output field {} missing", field_name))
        })?;

        // 如果有字段路径，使用 set_field
        if let Some(field_path) = &mapping.field_path {
            cache
                .set_field(&mapping.cache_key, field_path, field_value)
                .await?;
        } else {
            cache
                .set_raw(&mapping.cache_key, field_value.clone(), None)
                .await?;
        }
    }

    Ok(())
}

/// 构建输出对象（用于手动绑定函数输出到字段名）
///
/// 示例：
/// build_output_object(vec![
///   ("a", json!(a_value)),
///   ("b", json!(b_value)),
///   ("c", json!(c_value)),
/// ])
pub fn build_output_object(bindings: Vec<(String, JsonValue)>) -> JsonValue {
    let mut obj = serde_json::Map::new();
    for (field, value) in bindings {
        obj.insert(field, value);
    }
    JsonValue::Object(obj)
}
