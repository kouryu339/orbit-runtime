//! 元数据导出功能
//!
//! 集成类型验证，确保只导出可完全解构为基础类型的复合类型

use super::NodeRegistry;
use crate::data_type::DataTypeRegistry;
use serde_json::{json, Value};
use std::collections::HashMap;

// 引入类型结构验证器
#[cfg(feature = "type_structure")]
use crate::data_type::type_structure::TypeValidator;

/// 提取类型名称中的基础类型（去除 Array<>, Option<>, Vec<> 等包装）
#[allow(dead_code)]
fn extract_base_type(type_name: &str) -> String {
    let type_name = type_name.trim();

    // 处理 Array<T>, Vec<T>, Option<T> 等泛型
    if let Some(start) = type_name.find('<') {
        if let Some(end) = type_name.rfind('>') {
            let inner = &type_name[start + 1..end];
            return extract_base_type(inner); // 递归处理嵌套
        }
    }

    type_name.to_string()
}

/// 判断是否是泛型占位符或内置特殊类型
#[allow(dead_code)]
fn is_generic_or_builtin(type_name: &str) -> bool {
    matches!(
        type_name,
        "" |  // 执行引脚（无数据类型）
        "T" | "U" | "V" | // 泛型占位符
        "Any" | "Null" | "Void" | // 特殊类型
        "Array" | "Object" | // JSON 通用类型
        "i64" | "u64" | "f64" | "bool" | "String" | // 64位类型（统一标准）
        "DataValue" | "KeyValuePair" // 动态值和键值对类型
    )
}

/// 导出所有元数据到 JSON（带类型验证）
pub fn export_metadata(type_registry: &DataTypeRegistry) -> HashMap<String, Value> {
    let mut result = HashMap::new();

    // 导出节点
    let nodes: Vec<Value> = NodeRegistry::all()
        .into_iter()
        .map(|node| {
            json!({
                "name": node.node_type,
                "version": node.version,  // 添加版本号
                "display_name": node.display_name,
                "category": node.category,
                "description": node.description,
                "pins": node.pins,
                "permissions": node.permissions.bits,
                "wildcard_constraints": node.wildcard_constraints,
            })
        })
        .collect();

    // 导出基础类型（从框架的 DataTypeRegistry）
    let registered_types = type_registry.registered_types();
    let types: Vec<Value> = registered_types
        .into_iter()
        .map(|type_name| {
            let description = type_registry
                .type_description(&type_name)
                .unwrap_or_else(|| type_name.clone());
            json!({
                "name": type_name,
                "description": description,
            })
        })
        .collect();

    result.insert("nodes".to_string(), json!(nodes));
    result.insert("types".to_string(), json!(types));

    result
}

/// 导出所有元数据到 JSON（带类型结构验证）
#[cfg(feature = "type_structure")]
pub fn export_metadata_with_validation(
    type_registry: &DataTypeRegistry,
    type_validator: &TypeValidator,
) -> HashMap<String, Value> {
    let mut result = HashMap::new();

    // 导出节点（应用类型规范化）
    let nodes: Vec<Value> = NodeRegistry::all()
        .into_iter()
        .map(|node| {
            // 规范化引脚类型（PathBuf -> String, 32-bit -> 64-bit）
            let normalized_pins: Vec<Value> = node
                .pins
                .iter()
                .map(|pin| {
                    let normalized_data_type = type_validator.normalize_type(pin.data_type);
                    json!({
                        "name": pin.name,
                        "kind": pin.kind,
                        "data_type": normalized_data_type,
                        "description": pin.description,
                    })
                })
                .collect();

            json!({
                "name": node.node_type,
                "version": node.version,  // 添加版本号
                "display_name": node.display_name,
                "category": node.category,
                "description": node.description,
                "pins": normalized_pins,
                "permissions": node.permissions.bits,
                "wildcard_constraints": node.wildcard_constraints,
            })
        })
        .collect();

    // 导出基础类型
    let registered_types = type_registry.registered_types();
    let types: Vec<Value> = registered_types
        .into_iter()
        .map(|type_name| {
            let description = type_registry
                .type_description(&type_name)
                .unwrap_or_else(|| type_name.clone());
            json!({
                "name": type_name,
                "description": description,
            })
        })
        .collect();

    // ===== 关键：验证并导出类型结构（仅导出标记为可导出的类型）=====
    let all_type_structures = type_validator.get_all_type_structures();
    let total_types = all_type_structures.len();
    let exportable_types = type_validator.get_exportable_types();
    let filtered_count = total_types - exportable_types.len();

    let mut validated_structures = Vec::new();
    let mut validation_errors = Vec::new();

    tracing::debug!("🔍 类型过滤统计:");
    tracing::debug!("  📦 总注册类型: {} 个", total_types);
    tracing::debug!("  🔒 内部类型（不导出）: {} 个", filtered_count);
    tracing::debug!("  📤 可导出类型: {} 个\n", exportable_types.len());

    for type_name in &exportable_types {
        match type_validator.validate_type_decomposition(type_name) {
            Ok(()) => {
                // 类型可完全解构，添加到导出列表
                if let Some(field_tree) = type_validator.get_field_tree(type_name) {
                    validated_structures.push(json!({
                        "name": type_name,
                        "field_tree": field_tree,
                        "validated": true,
                    }));
                    tracing::debug!("  ✅ '{}' - 验证通过，已导出", type_name);
                }
            }
            Err(missing_types) => {
                // 类型依赖缺失，记录错误
                validation_errors.push(json!({
                    "type_name": type_name,
                    "error": "missing_dependencies",
                    "missing_types": missing_types,
                }));
                tracing::warn!(
                    "  ❌ '{}' - 验证失败，缺少依赖: {:?}",
                    type_name,
                    missing_types
                );
            }
        }
    }

    result.insert("nodes".to_string(), json!(nodes));
    result.insert("types".to_string(), json!(types));
    result.insert("type_structures".to_string(), json!(validated_structures));
    result.insert("validation_errors".to_string(), json!(validation_errors));

    // 最终统计
    tracing::debug!("\n📊 导出结果统计:");
    tracing::debug!("  ✅ 成功导出: {} 个类型结构", validated_structures.len());
    tracing::debug!("  ❌ 验证失败: {} 个", validation_errors.len());
    tracing::debug!("  🎯 蓝图节点: {} 个", nodes.len());

    result
}

/// 导出到 JSON 字符串
pub fn export_to_json(type_registry: &DataTypeRegistry) -> String {
    let metadata = export_metadata(type_registry);
    serde_json::to_string_pretty(&metadata).unwrap_or_default()
}

/// 导出到 JSON 字符串（带类型验证）
#[cfg(feature = "type_structure")]
pub fn export_to_json_with_validation(
    type_registry: &DataTypeRegistry,
    type_validator: &TypeValidator,
) -> String {
    let metadata = export_metadata_with_validation(type_registry, type_validator);
    serde_json::to_string_pretty(&metadata).unwrap_or_default()
}

/// 导出到文件（无类型验证，向后兼容）
pub fn export_to_files(type_registry: &DataTypeRegistry, output_dir: &str) -> std::io::Result<()> {
    use std::fs;
    use std::path::Path;

    let output_path = Path::new(output_dir);
    fs::create_dir_all(output_path)?;

    // 导出节点
    let nodes: Vec<Value> = NodeRegistry::all()
        .into_iter()
        .map(|node| {
            json!({
                "name": node.node_type,
                "version": node.version,
                "display_name": node.display_name,
                "category": node.category,
                "description": node.description,
                "pins": node.pins,
                "permissions": node.permissions.bits,
                "wildcard_constraints": node.wildcard_constraints,
            })
        })
        .collect();

    let nodes_json = serde_json::to_string_pretty(&nodes)?;
    fs::write(output_path.join("nodes.json"), nodes_json)?;

    // 导出类型
    let registered_types = type_registry.registered_types();
    let types: Vec<Value> = registered_types
        .into_iter()
        .map(|type_name| {
            let description = type_registry
                .type_description(&type_name)
                .unwrap_or_else(|| type_name.clone());
            json!({
                "name": type_name,
                "description": description,
            })
        })
        .collect();

    let types_json = serde_json::to_string_pretty(&types)?;
    fs::write(output_path.join("types.json"), types_json)?;

    Ok(())
}

/// 导出到文件（带类型验证 - 严格模式）
///
/// 如果发现任何类型无法完全解构为基础类型，将返回错误并阻止导出。
/// 必须修复所有类型验证问题后才能成功导出。
#[cfg(feature = "type_structure")]
pub fn export_to_files_with_validation(
    type_registry: &DataTypeRegistry,
    type_validator: &TypeValidator,
    output_dir: &str,
) -> std::io::Result<()> {
    use std::fs;
    use std::path::Path;

    tracing::debug!("📝 准备导出元数据到: {}", output_dir);

    // ===== 第一步：统计和验证类型 =====
    tracing::debug!("\n🔍 类型过滤与验证...");

    let all_type_structures = type_validator.get_all_type_structures();
    let total_types = all_type_structures.len();
    let exportable_types = type_validator.get_exportable_types();
    let filtered_count = total_types - exportable_types.len();

    tracing::debug!("  📦 总注册类型: {} 个", total_types);
    tracing::debug!("  🔒 内部类型（不导出）: {} 个", filtered_count);
    tracing::debug!("  📤 可导出类型: {} 个", exportable_types.len());

    let mut validation_errors = Vec::new();
    let mut validated_count = 0;

    tracing::debug!("\n🔍 验证可导出类型（严格模式）...");
    for type_name in &exportable_types {
        match type_validator.validate_type_decomposition(type_name) {
            Ok(()) => {
                tracing::debug!("  ✅ {} - 可完全解构", type_name);
                validated_count += 1;
            }
            Err(missing_types) => {
                tracing::warn!("  ❌ {} - 缺少依赖: {:?}", type_name, missing_types);
                validation_errors.push((type_name.clone(), missing_types));
            }
        }
    }

    // ===== 如果有验证失败，立即终止 =====
    if !validation_errors.is_empty() {
        tracing::warn!(
            "\n❌ 类型验证失败！发现 {} 个无法完全解构的类型:\n",
            validation_errors.len()
        );

        for (type_name, missing_types) in &validation_errors {
            tracing::warn!("  • {} 缺少以下依赖类型:", type_name);
            for missing in missing_types {
                tracing::warn!("      - {}", missing);
            }
        }

        tracing::warn!("\n💡 解决方案:");
        tracing::warn!("  1. 检查上述缺失的类型是否已定义");
        tracing::warn!("  2. 使用 validator.register_type() 注册缺失的类型");
        tracing::warn!("  3. 确保所有依赖类型都可以解构为基础类型");
        tracing::warn!("  4. 修复后重新运行导出命令");

        tracing::warn!("\n🚫 导出已中止，请先解决类型验证问题！");

        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "类型验证失败: {} 个类型无法完全解构",
                validation_errors.len()
            ),
        ));
    }

    // ===== 验证通过，继续导出 =====
    tracing::debug!("\n✅ 所有类型验证通过 ({} 个复合类型)", validated_count);
    tracing::debug!("\n📦 开始导出元数据...");

    let output_path = Path::new(output_dir);
    fs::create_dir_all(output_path)?;

    // 导出节点（应用类型规范化）
    let nodes: Vec<Value> = NodeRegistry::all()
        .into_iter()
        .map(|node| {
            // 规范化引脚类型（PathBuf -> String, 32-bit -> 64-bit）
            let normalized_pins: Vec<Value> = node
                .pins
                .iter()
                .map(|pin| {
                    let normalized_data_type = type_validator.normalize_type(pin.data_type);
                    json!({
                        "name": pin.name,
                        "kind": pin.kind,
                        "data_type": normalized_data_type,
                        "description": pin.description,
                    })
                })
                .collect();

            json!({
                "name": node.node_type,
                "version": node.version,
                "display_name": node.display_name,
                "category": node.category,
                "description": node.description,
                "pins": normalized_pins,
                "permissions": node.permissions.bits,
                "wildcard_constraints": node.wildcard_constraints,
            })
        })
        .collect();

    let nodes_json = serde_json::to_string_pretty(&nodes)?;
    fs::write(output_path.join("nodes.json"), nodes_json)?;
    tracing::debug!("  ✓ nodes.json ({} 个节点)", nodes.len());

    // ===== 第二步：引脚类型完整性验证 =====
    tracing::debug!("\n🔍 验证节点引脚类型完整性...");

    let registered_types = type_registry.registered_types();

    // 收集所有导出类型（基础类型 + 可导出复合类型）
    let mut all_exported_types: std::collections::HashSet<String> =
        registered_types.iter().cloned().collect();
    for type_name in &exportable_types {
        all_exported_types.insert(type_name.clone());
    }

    // 收集所有节点引脚使用的类型
    let mut pin_types_used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut missing_types: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    for node_metadata in NodeRegistry::all() {
        for pin in node_metadata.pins {
            let raw_type = pin.data_type;

            // 跳过非数据引脚（控制流引脚、空类型、泛型）
            use crate::workflow::registry::node_registry::PinKind;
            if raw_type.is_empty()
                || raw_type == "T"
                || raw_type == "U"
                || matches!(pin.kind, PinKind::ExecInput | PinKind::ExecOutput)
            {
                continue;
            }

            pin_types_used.insert(raw_type.to_string());

            // 使用 TypeValidator 验证类型是否可以完全解构
            if let Err(missing_deps) = type_validator.validate_type_decomposition(raw_type) {
                // 记录无法解构的类型
                let error_msg = if missing_deps.is_empty() {
                    format!("{} (引脚: {})", raw_type, pin.name)
                } else {
                    format!(
                        "{} (引脚: {}) - 缺少: {:?}",
                        raw_type, pin.name, missing_deps
                    )
                };

                missing_types
                    .entry(node_metadata.node_type.to_string())
                    .or_default()
                    .push(error_msg);
            }
        }
    }

    // 如果有缺失类型，报错并终止
    if !missing_types.is_empty() {
        tracing::warn!(
            "\n❌ 引脚类型验证失败！发现 {} 个节点使用了未导出的类型:\n",
            missing_types.len()
        );

        for (node_name, type_list) in &missing_types {
            tracing::warn!("  • 节点 {} 使用了未导出的类型:", node_name);
            for type_info in type_list {
                tracing::warn!("      - {}", type_info);
            }
        }

        tracing::warn!("\n💡 解决方案:");
        tracing::warn!("  1. 为缺失的依赖类型添加 #[buns_model(..., exportable = true)]");
        tracing::warn!("  2. 或将复杂嵌套类型（元组、HashMap等）封装为结构体");
        tracing::warn!("  3. 确保所有字段类型都可以递归解构为基础类型");
        tracing::warn!("  4. 修复后重新运行导出命令");

        tracing::warn!("\n🚫 导出已中止，请先解决类型解构问题！");

        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "引脚类型验证失败: {} 个节点使用了无法解构的类型",
                missing_types.len()
            ),
        ));
    }

    tracing::debug!(
        "  ✅ 所有引脚类型均可完全解构 ({} 种类型)",
        pin_types_used.len()
    );

    // 导出基础类型
    let types: Vec<Value> = registered_types
        .into_iter()
        .map(|type_name| {
            let description = type_registry
                .type_description(&type_name)
                .unwrap_or_else(|| type_name.clone());
            json!({
                "name": type_name,
                "description": description,
            })
        })
        .collect();

    let types_json = serde_json::to_string_pretty(&types)?;
    fs::write(output_path.join("types.json"), types_json)?;
    tracing::debug!("  ✓ types.json ({} 个基础类型)", types.len());

    // 导出可导出的类型结构定义（仅包含标记为 exportable 的类型）
    let exportable_structures: Vec<_> = all_type_structures
        .into_iter()
        .filter(|s| s.exportable)
        .collect();

    if !exportable_structures.is_empty() {
        let types_full_json = serde_json::to_string_pretty(&exportable_structures)?;
        fs::write(output_path.join("type_structures.json"), types_full_json)?;
        tracing::debug!(
            "  ✓ type_structures.json ({} 个可导出类型)",
            exportable_structures.len()
        );
    }

    // 导出字段树（单层展开，用于UI生成）
    let mut field_trees = Vec::new();

    for type_name in &exportable_types {
        if let Some(field_tree) = type_validator.get_field_tree(type_name) {
            field_trees.push(json!({
                "name": type_name,
                "field_tree": field_tree,
            }));
        }
    }

    if !field_trees.is_empty() {
        let field_trees_json = serde_json::to_string_pretty(&field_trees)?;
        fs::write(output_path.join("field_trees.json"), field_trees_json)?;
        tracing::debug!("  ✓ field_trees.json ({} 个字段树)", field_trees.len());
    }

    tracing::debug!("\n✅ 元数据导出完成！");
    tracing::debug!("📊 导出统计:");
    tracing::debug!("   节点: {}", nodes.len());
    tracing::debug!("   基础类型: {}", types.len());
    tracing::debug!(
        "   类型定义: {} (总注册 {}, 过滤 {})",
        exportable_structures.len(),
        total_types,
        filtered_count
    );
    tracing::debug!("   字段树: {}", field_trees.len());
    tracing::debug!("   验证状态: 全部通过 ✅");

    Ok(())
}
