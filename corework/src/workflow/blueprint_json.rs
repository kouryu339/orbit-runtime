//! 蓝图 JSON 格式定义
//!
//! 参考: BLUEPRINT_JSON_FORMAT_SPEC.md

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::Path;

pub const WORKFLOW_FORMAT: &str = "corework.workflow";
pub const WORKFLOW_SCHEMA_VERSION: &str = "1.0";
pub const WORKFLOW_FILE_SUFFIX: &str = ".workflow.json";
pub const DEFAULT_NODE_WIDTH: f64 = 240.0;
pub const DEFAULT_NODE_BASE_HEIGHT: f64 = 48.0;
pub const DEFAULT_PIN_ROW_HEIGHT: f64 = 24.0;

fn default_workflow_format() -> String {
    WORKFLOW_FORMAT.to_string()
}

fn default_schema_version() -> String {
    WORKFLOW_SCHEMA_VERSION.to_string()
}

fn default_min_runtime_version() -> String {
    "0.0.0".to_string()
}

fn current_runtime_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn parse_version_triplet(version: &str) -> Option<[u64; 3]> {
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some([major, minor, patch])
}

fn runtime_satisfies_minimum(current: &str, minimum: &str) -> bool {
    match (
        parse_version_triplet(current),
        parse_version_triplet(minimum),
    ) {
        (Some(current), Some(minimum)) => current >= minimum,
        _ => false,
    }
}

/// 蓝图可见性
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlueprintVisibility {
    /// 私有：用户自己创建的，可修改、可删除
    Private,
    Public,
}

impl Default for BlueprintVisibility {
    fn default() -> Self {
        Self::Private
    }
}

/// 蓝图元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintMetadata {
    /// 工作流稳定 ID。工作流重命名时不改变。
    #[serde(default)]
    pub id: String,

    /// 蓝图名称
    pub name: String,

    /// 创建时间 (ISO 8601)
    #[serde(default)]
    pub created: String,

    /// 最后修改时间 (ISO 8601)
    #[serde(default)]
    pub modified: String,

    /// 描述
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,

    /// 作者
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,

    /// 标签
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// 可见性：private（可修改）或 public（不可修改）
    #[serde(default)]
    pub visibility: BlueprintVisibility,

    /// 输入参数定义（从 StartNode 的 DataOutput 引脚提取）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<PinMetadata>,

    /// 输出参数定义（从 EndNode 的 DataInput 引脚提取）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<PinMetadata>,
}

/// 引脚元数据 - 用于蓝图元数据中描述输入/输出参数
///
/// 这是简化版本，只包含设计时需要的信息，不包含运行时字段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinMetadata {
    /// 引脚名称
    pub name: String,

    /// 数据类型
    pub data_type: String,

    /// 引脚描述
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,

    /// 默认值 (对于输入引脚)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<JsonValue>,
}

/// 节点引脚 - 完整的运行时引脚定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePin {
    /// 引脚名称
    pub name: String,

    /// 引脚类型: "ExecInput" | "ExecOutput" | "DataInput" | "DataOutput"
    pub kind: String,

    /// 数据类型 (对于数据引脚)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data_type: String,

    /// 引脚描述
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,

    /// 默认值 (对于输入引脚)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<JsonValue>,

    /// 解析后的类型信息 (运行时)
    #[serde(default, skip_serializing)]
    pub resolved_type: Option<JsonValue>,

    /// 拆分配置 - 类似UE的Split Struct Pin功能
    /// 当用户选择拆分聚合类型引脚时，此字段记录拆分信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_config: Option<SplitConfig>,
}

/// 拆分配置 - 记录引脚拆分状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitConfig {
    /// 是否已拆分
    pub is_split: bool,

    /// 拆分后的子引脚
    pub split_fields: Vec<SplitField>,
}

/// 拆分字段 - 描述拆分后的子引脚
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitField {
    /// 字段名（在结构体中的名称）
    pub field_name: String,

    /// 引脚名（在节点上显示的名称，通常为 "parent.field"）
    pub pin_name: String,

    /// 字段类型
    pub data_type: String,
}

/// 节点位置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
}

impl Default for NodePosition {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

/// 节点尺寸
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSize {
    pub width: f64,
    pub height: f64,
}

impl NodeSize {
    pub fn from_pins(pins: &[NodePin]) -> Self {
        let input_pin_count = pins
            .iter()
            .filter(|pin| pin.kind == "ExecInput" || pin.kind == "DataInput")
            .count();
        let output_pin_count = pins
            .iter()
            .filter(|pin| pin.kind == "ExecOutput" || pin.kind == "DataOutput")
            .count();
        let row_count = input_pin_count.max(output_pin_count) as f64;

        Self {
            width: DEFAULT_NODE_WIDTH,
            height: DEFAULT_NODE_BASE_HEIGHT + row_count * DEFAULT_PIN_ROW_HEIGHT,
        }
    }

    fn is_missing(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }
}

impl Default for NodeSize {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
        }
    }
}

/// 蓝图节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintNodeJson {
    /// 节点唯一 ID
    pub id: String,

    /// 节点类型 (如 "StartNode", "BranchNode")
    pub node_type: String,

    /// 节点位置
    #[serde(default)]
    pub position: NodePosition,

    /// 节点尺寸。旧 JSON 缺失时按引脚数量补齐。
    #[serde(default)]
    pub size: NodeSize,

    /// 节点引脚列表
    #[serde(default)]
    pub pins: Vec<NodePin>,

    /// 节点属性 (可选)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub properties: HashMap<String, JsonValue>,

    /// UI 显示名称 (可选)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// 注释 (可选)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// 连接
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionJson {
    /// 连接唯一 ID
    #[serde(default)]
    pub id: String,

    /// 源节点 ID (兼容Python的from_node)
    #[serde(alias = "from_node")]
    pub source_node: String,

    /// 源引脚名称 (兼容Python的from_pin)
    #[serde(alias = "from_pin")]
    pub source_pin: String,

    /// 目标节点 ID (兼容Python的to_node)
    #[serde(alias = "to_node")]
    pub target_node: String,

    /// 目标引脚名称 (兼容Python的to_pin)
    #[serde(alias = "to_pin")]
    pub target_pin: String,

    /// 连接类型: "Exec" | "Data"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub connection_type: String,
}

/// 蓝图变量
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintVariable {
    /// 变量名称
    pub name: String,

    /// 数据类型
    pub data_type: String,

    /// 默认值
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<JsonValue>,

    /// 变量描述
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// 注释框
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentBox {
    /// 注释 ID
    pub id: String,

    /// 注释文本
    pub text: String,

    /// 位置
    pub position: NodePosition,

    /// 大小
    pub size: CommentSize,

    /// 颜色 (可选)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// 注释框大小
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentSize {
    pub width: f64,
    pub height: f64,
}

/// 蓝图 JSON 格式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintJson {
    /// 文件类型标识
    #[serde(default = "default_workflow_format")]
    pub format: String,

    /// 持久化 schema 版本。兼容读取旧字段 `version`。
    #[serde(default = "default_schema_version", alias = "version")]
    pub schema_version: String,

    /// 执行该工作流所需的最低 FFI runtime 版本
    #[serde(default = "default_min_runtime_version")]
    pub min_runtime_version: String,

    /// 元数据
    pub metadata: BlueprintMetadata,

    /// 节点列表
    #[serde(default)]
    pub nodes: Vec<BlueprintNodeJson>,

    /// 连接列表
    #[serde(default)]
    pub connections: Vec<ConnectionJson>,

    /// 变量列表 (可选)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<BlueprintVariable>,

    /// 注释列表 (可选)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<CommentBox>,
}

impl BlueprintJson {
    /// 创建新的空蓝图
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            format: WORKFLOW_FORMAT.to_string(),
            schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
            min_runtime_version: current_runtime_version(),
            metadata: BlueprintMetadata {
                id: name.clone(),
                name,
                created: chrono::Utc::now().to_rfc3339(),
                modified: chrono::Utc::now().to_rfc3339(),
                description: String::new(),
                author: String::new(),
                tags: Vec::new(),
                visibility: BlueprintVisibility::Private,
                inputs: Vec::new(),
                outputs: Vec::new(),
            },
            nodes: Vec::new(),
            connections: Vec::new(),
            variables: Vec::new(),
            comments: Vec::new(),
        }
    }

    /// 从 JSON 字符串加载
    pub fn from_json_str(json_str: &str) -> Result<Self, serde_json::Error> {
        let mut blueprint: Self = serde_json::from_str(json_str)?;
        blueprint.normalize_node_sizes();
        Ok(blueprint)
    }

    /// 从 JSON 值加载
    pub fn from_json_value(json_value: JsonValue) -> Result<Self, serde_json::Error> {
        let mut blueprint: Self = serde_json::from_value(json_value)?;
        blueprint.normalize_node_sizes();
        Ok(blueprint)
    }

    /// 转换为 JSON 字符串
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// 转换为 JSON 值
    pub fn is_workflow_file_path(path: impl AsRef<Path>) -> bool {
        path.as_ref()
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(WORKFLOW_FILE_SUFFIX))
            .unwrap_or(false)
    }

    pub fn from_workflow_file(path: impl AsRef<Path>) -> std::result::Result<Self, String> {
        let path = path.as_ref();
        if !Self::is_workflow_file_path(path) {
            return Err(format!(
                "workflow file must end with `{}`: {}",
                WORKFLOW_FILE_SUFFIX,
                path.display()
            ));
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read workflow file {}: {}", path.display(), e))?;
        let blueprint =
            Self::from_json_str(&content).map_err(|e| format!("invalid workflow JSON: {}", e))?;
        blueprint.validate()?;
        Ok(blueprint)
    }

    pub fn save_to_workflow_file(
        &mut self,
        path: impl AsRef<Path>,
    ) -> std::result::Result<(), String> {
        let path = path.as_ref();
        if !Self::is_workflow_file_path(path) {
            return Err(format!(
                "workflow file must end with `{}`: {}",
                WORKFLOW_FILE_SUFFIX,
                path.display()
            ));
        }

        self.format = WORKFLOW_FORMAT.to_string();
        self.schema_version = WORKFLOW_SCHEMA_VERSION.to_string();
        self.normalize_node_sizes();
        self.update_modified_time();
        self.validate()?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "failed to create workflow directory {}: {}",
                    parent.display(),
                    e
                )
            })?;
        }
        let json = self
            .to_json_string()
            .map_err(|e| format!("failed to serialize workflow JSON: {}", e))?;
        std::fs::write(path, json)
            .map_err(|e| format!("failed to write workflow file {}: {}", path.display(), e))
    }

    pub fn to_json_value(&self) -> Result<JsonValue, serde_json::Error> {
        serde_json::to_value(self)
    }

    /// 查找节点
    pub fn find_node(&self, node_id: &str) -> Option<&BlueprintNodeJson> {
        self.nodes.iter().find(|n| n.id == node_id)
    }

    /// 查找节点（可变引用）
    pub fn find_node_mut(&mut self, node_id: &str) -> Option<&mut BlueprintNodeJson> {
        self.nodes.iter_mut().find(|n| n.id == node_id)
    }

    /// 添加节点
    pub fn add_node(&mut self, mut node: BlueprintNodeJson) {
        if node.size.is_missing() {
            node.size = NodeSize::from_pins(&node.pins);
        }
        self.nodes.push(node);
        self.update_modified_time();
    }

    /// 移除节点
    pub fn remove_node(&mut self, node_id: &str) -> Option<BlueprintNodeJson> {
        if let Some(index) = self.nodes.iter().position(|n| n.id == node_id) {
            self.update_modified_time();
            Some(self.nodes.remove(index))
        } else {
            None
        }
    }

    /// 添加连接
    pub fn add_connection(&mut self, connection: ConnectionJson) {
        self.connections.push(connection);
        self.update_modified_time();
    }

    /// 移除连接
    pub fn remove_connections_for_node(&mut self, node_id: &str) {
        self.connections
            .retain(|c| c.source_node != node_id && c.target_node != node_id);
        self.update_modified_time();
    }

    /// 更新修改时间
    pub fn update_modified_time(&mut self) {
        self.metadata.modified = chrono::Utc::now().to_rfc3339();
    }

    /// 为旧 JSON 或新生成节点补齐可视化尺寸。
    pub fn normalize_node_sizes(&mut self) {
        for node in &mut self.nodes {
            if node.size.is_missing() {
                node.size = NodeSize::from_pins(&node.pins);
            }
        }
    }

    /// 验证蓝图结构
    pub fn validate(&self) -> Result<(), String> {
        // 检查版本
        if self.format != WORKFLOW_FORMAT {
            return Err(format!("不支持的 workflow format: {}", self.format));
        }
        if self.schema_version != WORKFLOW_SCHEMA_VERSION {
            return Err(format!(
                "不支持的 workflow schema_version: {}",
                self.schema_version
            ));
        }
        if self.min_runtime_version.is_empty() {
            return Err("缺少 min_runtime_version 字段".to_string());
        }
        let current_runtime = current_runtime_version();
        if !runtime_satisfies_minimum(&current_runtime, &self.min_runtime_version) {
            return Err(format!(
                "runtime 版本过低: 当前 {}, 工作流最低要求 {}",
                current_runtime, self.min_runtime_version
            ));
        }

        // 检查节点 ID 唯一性
        let mut node_ids = std::collections::HashSet::new();
        for node in &self.nodes {
            if !node_ids.insert(&node.id) {
                return Err(format!("节点 ID 重复: {}", node.id));
            }
            if !node.position.x.is_finite() || !node.position.y.is_finite() {
                return Err(format!("节点坐标非法: {}", node.id));
            }
            if !node.size.width.is_finite()
                || !node.size.height.is_finite()
                || node.size.width <= 0.0
                || node.size.height <= 0.0
            {
                return Err(format!("节点尺寸非法: {}", node.id));
            }
        }

        // 检查连接引用的节点存在
        let mut connection_ids = std::collections::HashSet::new();
        for conn in &self.connections {
            if !conn.id.is_empty() && !connection_ids.insert(&conn.id) {
                return Err(format!("连接 ID 重复: {}", conn.id));
            }
            if !node_ids.contains(&conn.source_node) {
                return Err(format!("连接引用的源节点不存在: {}", conn.source_node));
            }
            if !node_ids.contains(&conn.target_node) {
                return Err(format!("连接引用的目标节点不存在: {}", conn.target_node));
            }

            let source_node = self.find_node(&conn.source_node).expect("checked above");
            if !Self::node_has_pin(source_node, &conn.source_pin) {
                return Err(format!(
                    "连接引用的源引脚不存在: {}.{}",
                    conn.source_node, conn.source_pin
                ));
            }
            let target_node = self.find_node(&conn.target_node).expect("checked above");
            if !Self::node_has_pin(target_node, &conn.target_pin) {
                return Err(format!(
                    "连接引用的目标引脚不存在: {}.{}",
                    conn.target_node, conn.target_pin
                ));
            }
        }

        let mut variable_names = std::collections::HashSet::new();
        for variable in &self.variables {
            if !variable_names.insert(&variable.name) {
                return Err(format!("变量名称重复: {}", variable.name));
            }
        }

        Ok(())
    }

    fn node_has_pin(node: &BlueprintNodeJson, pin_name: &str) -> bool {
        node.pins.iter().any(|pin| {
            pin.name == pin_name
                || pin
                    .split_config
                    .as_ref()
                    .map(|config| {
                        config
                            .split_fields
                            .iter()
                            .any(|field| field.pin_name == pin_name)
                    })
                    .unwrap_or(false)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_empty_blueprint() {
        let bp = BlueprintJson::new("测试蓝图");
        assert_eq!(bp.schema_version, "1.0");
        assert_eq!(bp.metadata.name, "测试蓝图");
        assert!(bp.nodes.is_empty());
        assert!(bp.connections.is_empty());
    }

    #[test]
    fn test_add_node() {
        let mut bp = BlueprintJson::new("测试");
        let node = BlueprintNodeJson {
            id: "node_1".to_string(),
            node_type: "StartNode".to_string(),
            position: NodePosition { x: 100.0, y: 200.0 },
            size: NodeSize::from_pins(&[]),
            pins: vec![],
            properties: HashMap::new(),
            display_name: None,
            comment: None,
        };

        bp.add_node(node);
        assert_eq!(bp.nodes.len(), 1);
        assert_eq!(bp.find_node("node_1").unwrap().node_type, "StartNode");
    }

    #[test]
    fn test_serialize_deserialize() {
        let mut bp = BlueprintJson::new("测试蓝图");
        bp.add_node(BlueprintNodeJson {
            id: "node_1".to_string(),
            node_type: "StartNode".to_string(),
            position: NodePosition::default(),
            size: NodeSize::from_pins(&[]),
            pins: vec![NodePin {
                name: "Out".to_string(),
                kind: "ExecOutput".to_string(),
                data_type: String::new(),
                description: String::new(),
                default_value: None,
                resolved_type: None,
                split_config: None,
            }],
            properties: HashMap::new(),
            display_name: None,
            comment: None,
        });

        // 序列化
        let json_str = bp.to_json_string().unwrap();

        // 反序列化
        let loaded_bp = BlueprintJson::from_json_str(&json_str).unwrap();
        assert_eq!(loaded_bp.nodes.len(), 1);
        assert_eq!(loaded_bp.nodes[0].pins.len(), 1);
    }

    #[test]
    fn test_validate() {
        let mut bp = BlueprintJson::new("测试");
        let pins = vec![
            NodePin {
                name: "In".to_string(),
                kind: "ExecInput".to_string(),
                data_type: String::new(),
                description: String::new(),
                default_value: None,
                resolved_type: None,
                split_config: None,
            },
            NodePin {
                name: "Out".to_string(),
                kind: "ExecOutput".to_string(),
                data_type: String::new(),
                description: String::new(),
                default_value: None,
                resolved_type: None,
                split_config: None,
            },
        ];

        // 添加节点
        bp.add_node(BlueprintNodeJson {
            id: "node_1".to_string(),
            node_type: "StartNode".to_string(),
            position: NodePosition::default(),
            size: NodeSize::from_pins(&pins),
            pins,
            properties: HashMap::new(),
            display_name: None,
            comment: None,
        });

        // 添加有效连接
        bp.add_connection(ConnectionJson {
            id: "conn_1".to_string(),
            source_node: "node_1".to_string(),
            source_pin: "Out".to_string(),
            target_node: "node_1".to_string(),
            target_pin: "In".to_string(),
            connection_type: "Exec".to_string(),
        });

        assert!(bp.validate().is_ok());

        // 添加无效连接
        bp.add_connection(ConnectionJson {
            id: "conn_2".to_string(),
            source_node: "node_999".to_string(),
            source_pin: "Out".to_string(),
            target_node: "node_1".to_string(),
            target_pin: "In".to_string(),
            connection_type: "Exec".to_string(),
        });

        assert!(bp.validate().is_err());
    }
}
