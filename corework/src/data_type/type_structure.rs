//! 类型结构定义和验证
//!
//! 支持复合类型的字段定义和递归类型检查

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, Serialize)]
pub struct FieldDefinition {
    pub name: &'static str,
    pub type_name: &'static str, // 字段类型（可以是基础类型或自定义类型）
    pub description: &'static str,
    pub optional: bool, // 是否可选
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct EnumVariant {
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TypeStructure {
    pub name: &'static str,
    pub version: &'static str, // 类型版本号（语义版本："1.0.0"）
    pub description: &'static str,
    pub category: &'static str, // 类型分类（如 "OCR", "Grading", "Common"）
    pub fields: &'static [FieldDefinition],
    pub is_primitive: bool,                    // 是否为基础类型
    pub exportable: bool, // 是否导出到蓝图编辑器（默认true，内部类型可设为false）
    pub is_enum: bool,    // 是否为枚举类型
    pub enum_variants: &'static [EnumVariant], // 枚举变体列表（仅用于枚举类型）
}

// 注册到 inventory 以便自动收集
inventory::collect!(TypeStructure);

/// 类型验证器 - 递归检查类型依赖
pub struct TypeValidator {
    /// 已注册的类型结构
    type_structures: HashMap<String, TypeStructure>,

    /// 基础类型集合
    primitive_types: HashSet<String>,
}

impl Default for TypeValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeValidator {
    pub fn new() -> Self {
        let mut primitive_types = HashSet::new();
        primitive_types.insert("String".to_string());
        primitive_types.insert("i32".to_string());
        primitive_types.insert("i64".to_string());
        primitive_types.insert("u32".to_string());
        primitive_types.insert("u64".to_string());
        primitive_types.insert("f32".to_string());
        primitive_types.insert("f64".to_string());
        primitive_types.insert("bool".to_string());
        primitive_types.insert("usize".to_string());
        primitive_types.insert("Int".to_string()); // 别名
        primitive_types.insert("Float".to_string()); // 别名
        primitive_types.insert("Bool".to_string()); // 别名

        // 标准库类型（在 Python 端映射为基础类型）
        primitive_types.insert("PathBuf".to_string()); // 文件路径 -> str
        primitive_types.insert("std::path::PathBuf".to_string()); // 完整路径

        // JSON/Object 类型（在 Python 端映射为 Object）
        primitive_types.insert("Object".to_string()); // Object 类型
        primitive_types.insert("Value".to_string()); // serde_json::Value -> Object
        primitive_types.insert("serde_json::Value".to_string()); // 完整路径 -> Object

        // 键值对类型（用于对象构造）
        primitive_types.insert("KeyValuePair".to_string()); // 键值对类型

        Self {
            type_structures: HashMap::new(),
            primitive_types,
        }
    }

    /// 注册类型结构
    pub fn register_type(&mut self, structure: TypeStructure) {
        let name = structure.name.to_string();
        if structure.is_primitive {
            self.primitive_types.insert(name.clone());
        }
        self.type_structures.insert(name, structure);
    }

    /// 递归处理容器类型（Vec<PathBuf> -> Vec<String>）
    pub fn normalize_type(&self, type_name: &str) -> String {
        // 处理容器类型 Vec<T>, Array<T>, Option<T>
        if let Some(start) = type_name.find('<') {
            if let Some(end) = type_name.rfind('>') {
                let container = &type_name[..start];
                let inner = &type_name[start + 1..end];

                // 递归规范化内部类型
                let normalized_inner = self.normalize_type(inner);
                return format!("{}<{}>", container, normalized_inner);
            }
        }

        // 处理基础类型映射
        match type_name {
            "u32" => "u64".to_string(),
            "i32" => "i64".to_string(),
            "f32" => "f64".to_string(),
            "usize" => "usize".to_string(), // usize 保持不变
            "PathBuf" => "String".to_string(),
            "std::path::PathBuf" => "String".to_string(),
            _ => type_name.to_string(),
        }
    }

    /// 检查类型是否为基础类型
    pub fn is_primitive(&self, type_name: &str) -> bool {
        // 先应用类型映射
        let normalized = self.normalize_type(type_name);

        // 处理容器类型
        if normalized.starts_with("Array<") || normalized.starts_with("Vec<") {
            // 提取元素类型
            if let Some(element_type) = self.extract_container_element(&normalized) {
                return self.is_primitive(&element_type);
            }
            return false;
        }

        if normalized.starts_with("Option<") {
            if let Some(inner_type) = self.extract_container_element(&normalized) {
                return self.is_primitive(&inner_type);
            }
            return false;
        }

        self.primitive_types.contains(&normalized)
    }

    /// 提取容器元素类型
    fn extract_container_element(&self, type_name: &str) -> Option<String> {
        if let Some(start) = type_name.find('<') {
            if let Some(end) = type_name.rfind('>') {
                if end > start {
                    return Some(type_name[start + 1..end].to_string());
                }
            }
        }
        None
    }

    /// 验证类型是否可以完全解构为基础类型
    ///
    /// 返回 Ok(()) 表示验证通过
    /// 返回 Err 包含缺失的类型名称
    pub fn validate_type_decomposition(&self, type_name: &str) -> Result<(), Vec<String>> {
        let mut visited = HashSet::new();
        let mut missing_types = Vec::new();

        self.validate_recursive(type_name, &mut visited, &mut missing_types);

        if missing_types.is_empty() {
            Ok(())
        } else {
            Err(missing_types)
        }
    }

    /// 递归验证类型
    fn validate_recursive(
        &self,
        type_name: &str,
        visited: &mut HashSet<String>,
        missing: &mut Vec<String>,
    ) {
        // 空字符串表示控制流引脚（无数据），直接通过
        if type_name.is_empty() {
            return;
        }

        // 应用类型映射：32位类型 -> 64位类型
        let normalized_type = self.normalize_type(type_name);

        // 泛型类型参数（T、U等）直接通过（在运行时确定）
        if normalized_type.len() == 1 && normalized_type.chars().next().unwrap().is_uppercase() {
            return;
        }

        // Any 类型（动态类型）直接通过
        if normalized_type == "Any" {
            return;
        }

        // 防止循环引用
        if visited.contains(&normalized_type) {
            return;
        }
        visited.insert(normalized_type.clone());

        // 处理容器类型
        if normalized_type.starts_with("Array<")
            || normalized_type.starts_with("Vec<")
            || normalized_type.starts_with("Option<")
        {
            if let Some(element_type) = self.extract_container_element(&normalized_type) {
                self.validate_recursive(&element_type, visited, missing);
            }
            return;
        }

        // 处理 HashMap
        if normalized_type.starts_with("HashMap<") {
            // HashMap 不支持导出，需要封装为结构体
            if !missing.contains(&type_name.to_string()) {
                missing.push(type_name.to_string());
            }
            return;
        }

        // 处理元组类型 (T, U, V)
        if normalized_type.starts_with('(') && normalized_type.ends_with(')') {
            // 元组不支持导出，需要封装为结构体
            if !missing.contains(&type_name.to_string()) {
                missing.push(type_name.to_string());
            }
            return;
        }

        // 基础类型直接通过
        if self.is_primitive(&normalized_type) {
            return;
        }

        // 查找类型定义
        if let Some(structure) = self.type_structures.get(normalized_type.as_str()) {
            // 枚举类型直接通过（不需要递归检查字段）
            if structure.is_enum {
                return;
            }

            // 递归检查所有字段
            for field in structure.fields {
                self.validate_recursive(field.type_name, visited, missing);
            }
        } else {
            // 类型未注册
            if !missing.contains(&type_name.to_string()) {
                missing.push(type_name.to_string());
            }
        }
    }

    /// 获取类型的完整字段树（递归展开所有层级）
    ///
    /// ⚠️ 警告：此方法会完全扁平化所有嵌套结构，可能导致结构性丢失
    pub fn get_field_tree_full(&self, type_name: &str) -> Option<FieldTree> {
        // 应用类型映射
        let normalized = self.normalize_type(type_name);

        if self.is_primitive(&normalized) {
            return Some(FieldTree::Primitive(normalized));
        }

        // 处理容器类型
        if let Some(element_type) = self.extract_container_element(type_name) {
            // 对元素类型也应用映射
            let normalized_element = self.normalize_type(&element_type);
            let element_tree = self.get_field_tree_full(&normalized_element)?;
            if type_name.starts_with("Array<") || type_name.starts_with("Vec<") {
                return Some(FieldTree::Array(Box::new(element_tree)));
            } else if type_name.starts_with("Option<") {
                return Some(FieldTree::Optional(Box::new(element_tree)));
            }
        }

        // 获取复合类型结构
        if let Some(structure) = self.type_structures.get(type_name) {
            let mut field_trees = Vec::new();

            for field in structure.fields {
                if let Some(field_tree) = self.get_field_tree_full(field.type_name) {
                    field_trees.push(FieldTreeNode {
                        name: field.name.to_string(),
                        description: field.description.to_string(),
                        optional: field.optional,
                        tree: field_tree,
                    });
                } else {
                    // 字段类型无法解析
                    return None;
                }
            }

            return Some(FieldTree::Composite {
                type_name: type_name.to_string(),
                fields: field_trees,
            });
        }

        None
    }

    /// 获取类型的字段树（仅展开一层）
    ///
    /// 对于嵌套的自定义类型，保留类型引用而不递归展开。
    /// 这样可以：
    /// 1. 保持子结构的完整性（不会过度扁平化）
    /// 2. 支持逐层拆分引脚（先拆A为A.b和A.c，再拆A.b为A.b.x和A.b.y）
    /// 3. 避免深层嵌套时UI过于复杂
    ///
    /// 示例：Rectangle { top_left: Point2D, bottom_right: Point2D }
    /// - 完全展开：top_left.x, top_left.y, bottom_right.x, bottom_right.y （丢失结构性）
    /// - 单层展开：top_left(Point2D), bottom_right(Point2D) （保持结构性）
    pub fn get_field_tree(&self, type_name: &str) -> Option<FieldTree> {
        // 先处理容器类型（在应用类型映射前）
        if type_name.starts_with("Array<")
            || type_name.starts_with("Vec<")
            || type_name.starts_with("Option<")
        {
            if let Some(element_type) = self.extract_container_element(type_name) {
                // 对元素类型应用映射并递归
                let normalized_element = self.normalize_type(&element_type);

                if self.is_primitive(&normalized_element) {
                    let element_tree = FieldTree::Primitive(normalized_element);
                    if type_name.starts_with("Array<") || type_name.starts_with("Vec<") {
                        return Some(FieldTree::Array(Box::new(element_tree)));
                    } else if type_name.starts_with("Option<") {
                        return Some(FieldTree::Optional(Box::new(element_tree)));
                    }
                } else {
                    // 自定义类型作为类型引用
                    let element_tree = FieldTree::TypeReference(normalized_element);
                    if type_name.starts_with("Array<") || type_name.starts_with("Vec<") {
                        return Some(FieldTree::Array(Box::new(element_tree)));
                    } else if type_name.starts_with("Option<") {
                        return Some(FieldTree::Optional(Box::new(element_tree)));
                    }
                }
            }
        }

        // 应用类型映射：32位 -> 64位
        let normalized = self.normalize_type(type_name);

        // 基础类型直接返回
        if self.is_primitive(&normalized) {
            return Some(FieldTree::Primitive(normalized));
        }

        // 获取复合类型结构（仅展开一层）
        if let Some(structure) = self.type_structures.get(type_name) {
            // 枚举类型作为基础类型处理（不展开字段）
            if structure.is_enum {
                return Some(FieldTree::Enum {
                    type_name: type_name.to_string(),
                    variants: structure
                        .enum_variants
                        .iter()
                        .map(|v| (v.name.to_string(), v.description.to_string()))
                        .collect(),
                });
            }

            let mut field_trees = Vec::new();

            for field in structure.fields {
                // 直接递归处理字段类型（包含容器和映射逻辑）
                let field_tree = if field.type_name.starts_with("Array<")
                    || field.type_name.starts_with("Vec<")
                    || field.type_name.starts_with("Option<")
                {
                    // 容器类型：递归调用 get_field_tree 处理（会自动应用映射）
                    self.get_field_tree(field.type_name)?
                } else {
                    // 非容器类型：应用类型映射后判断
                    let normalized_field_type = self.normalize_type(field.type_name);
                    if self.is_primitive(&normalized_field_type) {
                        // 基础类型直接记录
                        FieldTree::Primitive(normalized_field_type)
                    } else {
                        // 自定义类型：只保留类型引用，不展开其字段
                        FieldTree::TypeReference(normalized_field_type)
                    }
                };

                field_trees.push(FieldTreeNode {
                    name: field.name.to_string(),
                    description: field.description.to_string(),
                    optional: field.optional,
                    tree: field_tree,
                });
            }

            return Some(FieldTree::Composite {
                type_name: type_name.to_string(),
                fields: field_trees,
            });
        }

        None
    }

    /// 获取所有可导出的类型（已验证可完全解构且标记为可导出）
    pub fn get_exportable_types(&self) -> Vec<String> {
        self.type_structures
            .iter()
            .filter(|(type_name, structure)| {
                structure.exportable
                    && !structure.is_primitive
                    && self.validate_type_decomposition(type_name).is_ok()
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// 获取所有已注册的 TypeStructure
    pub fn get_all_type_structures(&self) -> Vec<TypeStructure> {
        self.type_structures.values().copied().collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldTree {
    Primitive(String),

    /// 类型引用（不展开字段，保持结构性）
    /// 用于单层拆分，避免过度扁平化
    TypeReference(String),

    /// 数组类型
    Array(Box<FieldTree>),

    /// 可选类型
    Optional(Box<FieldTree>),

    /// 枚举类型（显示为下拉选择框）
    /// 包含类型名和所有可选变体 (变体名, 变体描述)
    Enum {
        type_name: String,
        variants: Vec<(String, String)>,
    },

    /// 复合类型
    Composite {
        type_name: String,
        fields: Vec<FieldTreeNode>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldTreeNode {
    pub name: String,
    pub description: String,
    pub optional: bool,
    pub tree: FieldTree,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_types() {
        let validator = TypeValidator::new();

        // 测试基础类型识别
        assert!(validator.is_primitive("String"));
        assert!(validator.is_primitive("i32"));
        assert!(validator.is_primitive("f64"));
        assert!(validator.is_primitive("bool"));
        assert!(!validator.is_primitive("CustomType"));
    }

    #[test]
    fn test_container_types() {
        let validator = TypeValidator::new();

        // 测试容器类型
        assert!(validator.is_primitive("Array<String>"));
        assert!(validator.is_primitive("Vec<i32>"));
        assert!(!validator.is_primitive("Array<CustomType>"));
    }
}
