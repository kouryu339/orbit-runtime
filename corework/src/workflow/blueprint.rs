use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;

use crate::error::Result;
use crate::orchestration::Context;

// 使用新的 core 模块定义
pub use super::core::{
    Connection, DataValue, NodeOutput, Pin, PinCacheMapping, PinContainerType, PinDirection,
    PinType,
};
/// Blueprint Node Trait
#[async_trait]
pub trait BlueprintNode: Send + Sync + fmt::Debug {
    fn name(&self) -> &str;
    fn pins(&self) -> Vec<Pin>;

    async fn execute(
        &self,
        ctx: &Context,
        input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput>;

    fn validate_connections(&self, _connections: &[Connection]) -> Result<()> {
        Ok(())
    }
}

/// Entry node - workflow start point
#[derive(Debug)]
pub struct EntryNode {
    name: String,
}

impl EntryNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl BlueprintNode for EntryNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![Pin::exec_out("exec")]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        _input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        Ok(NodeOutput::ExecPin("exec".to_string()))
    }
}

/// Task node - execute system operation (legacy, simple version)
#[allow(dead_code)]
#[derive(Debug)]
pub struct TaskNode {
    name: String,
    system_name: String,
    operation_name: String,
}

impl TaskNode {
    pub fn new(
        name: impl Into<String>,
        system_name: impl Into<String>,
        operation_name: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            system_name: system_name.into(),
            operation_name: operation_name.into(),
        }
    }
}

#[async_trait]
impl BlueprintNode for TaskNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("exec"),
            Pin::exec_out("then"),
            Pin::data_in("input", "String"),
            Pin::data_out("output", "String"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        _input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        // For now, just return success
        // A real implementation would get the operation from registry and execute it
        // let operation = ctx.registry.get(&self.operation_name)?;

        let mut output = HashMap::new();
        output.insert(
            "output".to_string(),
            DataValue::from_string("Operation completed"),
        );

        Ok(NodeOutput::ExecPin("then".to_string()))
    }
}

/// 系统节点 - 通用的System调用节点，支持Cache自动读写
///
/// 使用方式：
/// ```
/// SystemNode::builder("ImageMerge")
///     .system_type_id(TypeId::of::<ImageMergeSystem>())
///     .input_mapping(PinCacheMapping::new("image_paths", "batch::image_paths", "Vec<PathBuf>"))
///     .output_mapping(PinCacheMapping::new("merged_data", "batch::merged_data", "Vec<MergedImageData>"))
///     .build()
/// ```
#[derive(Debug)]
pub struct SystemNode {
    name: String,
    /// 系统类型ID（用于从Context获取系统实例）
    #[allow(dead_code)]
    system_type_id: std::any::TypeId,
    /// 输入Pin映射（Pin名 → Cache Key）
    input_mappings: Vec<PinCacheMapping>,
    /// 输出Pin映射（Pin名 → Cache Key）
    output_mappings: Vec<PinCacheMapping>,
    /// 失败时的输出Pin（可选）
    error_pin: Option<String>,
}

impl SystemNode {
    pub fn builder(name: impl Into<String>) -> SystemNodeBuilder {
        SystemNodeBuilder::new(name)
    }
}

/// SystemNode构建器
pub struct SystemNodeBuilder {
    name: String,
    system_type_id: Option<std::any::TypeId>,
    input_mappings: Vec<PinCacheMapping>,
    output_mappings: Vec<PinCacheMapping>,
    error_pin: Option<String>,
}

impl SystemNodeBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            system_type_id: None,
            input_mappings: Vec::new(),
            output_mappings: Vec::new(),
            error_pin: None,
        }
    }

    /// 设置系统类型ID
    pub fn system_type_id(mut self, type_id: std::any::TypeId) -> Self {
        self.system_type_id = Some(type_id);
        self
    }

    /// 添加输入映射
    pub fn input_mapping(mut self, mapping: PinCacheMapping) -> Self {
        self.input_mappings.push(mapping);
        self
    }

    /// 添加输出映射
    pub fn output_mapping(mut self, mapping: PinCacheMapping) -> Self {
        self.output_mappings.push(mapping);
        self
    }

    /// 设置错误Pin（可选）
    pub fn error_pin(mut self, pin_name: impl Into<String>) -> Self {
        self.error_pin = Some(pin_name.into());
        self
    }

    pub fn build(self) -> Result<SystemNode> {
        let system_type_id = self
            .system_type_id
            .ok_or_else(|| anyhow::anyhow!("System type ID not set"))?;

        Ok(SystemNode {
            name: self.name,
            system_type_id,
            input_mappings: self.input_mappings,
            output_mappings: self.output_mappings,
            error_pin: self.error_pin,
        })
    }
}

#[async_trait]
impl BlueprintNode for SystemNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        let mut pins = vec![Pin::exec_in("exec"), Pin::exec_out("then")];

        // 添加错误输出Pin
        if self.error_pin.is_some() {
            pins.push(Pin::exec_out("error"));
        }

        // 添加输入数据Pins
        for mapping in &self.input_mappings {
            pins.push(Pin::data_in(&mapping.pin_name, &mapping.type_name));
        }

        // 添加输出数据Pins
        for mapping in &self.output_mappings {
            pins.push(Pin::data_out(&mapping.pin_name, &mapping.type_name));
        }

        pins
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        _input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        // 注意：这里需要使用unsafe或者重新设计来支持泛型System调用
        // 当前版本提供接口定义，实际调用需要在业务层通过辅助函数完成

        // 框架只负责：
        // 1. 验证Pin配置
        // 2. 提供统一的execute接口
        // 3. 标准化错误处理

        // 实际的System调用由业务层的辅助函数完成（见下面的helper）
        Err(anyhow::anyhow!("SystemNode needs typed helper for execution. Use SystemNodeHelper::execute_typed in your business code.").into())
    }
}

/// Branch node - conditional execution
#[derive(Debug)]
pub struct BranchNode {
    name: String,
}

impl BranchNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl BlueprintNode for BranchNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("exec"),
            Pin::exec_out("true"),
            Pin::exec_out("false"),
            Pin::data_in("condition", "bool"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let condition = input_data
            .get("condition")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let next_pin = if condition { "true" } else { "false" };
        Ok(NodeOutput::ExecPin(next_pin.to_string()))
    }
}

/// Sequence node - execute multiple outputs in order
#[derive(Debug)]
pub struct SequenceNode {
    name: String,
    output_count: usize,
}

impl SequenceNode {
    pub fn new(name: impl Into<String>, output_count: usize) -> Self {
        Self {
            name: name.into(),
            output_count,
        }
    }
}

#[async_trait]
impl BlueprintNode for SequenceNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        let mut pins = vec![Pin::exec_in("exec")];
        for i in 0..self.output_count {
            pins.push(Pin::exec_out(format!("then_{}", i)));
        }
        pins
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        _input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let outputs: Vec<String> = (0..self.output_count)
            .map(|i| format!("then_{}", i))
            .collect();
        Ok(NodeOutput::Multiple(outputs))
    }
}

/// For loop node - iterate over range
#[derive(Debug)]
pub struct ForLoopNode {
    name: String,
}

impl ForLoopNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl BlueprintNode for ForLoopNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("exec"),
            Pin::exec_out("loop_body"),
            Pin::exec_out("completed"),
            Pin::data_in("start", "i64"),
            Pin::data_in("end", "i64"),
            Pin::data_out("index", "i64"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let start = input_data
            .get("start")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let _end = input_data.get("end").and_then(|v| v.as_i64()).unwrap_or(0);

        // For simplicity, just execute loop body once in this example
        // A real implementation would need state management for iteration
        let mut output = HashMap::new();
        output.insert("index".to_string(), DataValue::from_i64(start));

        Ok(NodeOutput::ExecPin("loop_body".to_string()))
    }
}

/// Pure function node - data processing without side effects
pub struct PureFunctionNode {
    name: String,
    inputs: Vec<(String, String)>,  // (pin_name, type_name)
    outputs: Vec<(String, String)>, // (pin_name, type_name)
    function: Arc<dyn Fn(HashMap<String, DataValue>) -> HashMap<String, DataValue> + Send + Sync>,
}

impl fmt::Debug for PureFunctionNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PureFunctionNode")
            .field("name", &self.name)
            .field("inputs", &self.inputs)
            .field("outputs", &self.outputs)
            .field("function", &"<function>")
            .finish()
    }
}

impl PureFunctionNode {
    pub fn new(
        name: impl Into<String>,
        inputs: Vec<(String, String)>,
        outputs: Vec<(String, String)>,
        function: impl Fn(HashMap<String, DataValue>) -> HashMap<String, DataValue>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            inputs,
            outputs,
            function: Arc::new(function),
        }
    }
}

#[async_trait]
impl BlueprintNode for PureFunctionNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        let mut pins = Vec::new();
        for (pin_name, type_name) in &self.inputs {
            pins.push(Pin::data_in(pin_name, type_name));
        }
        for (pin_name, type_name) in &self.outputs {
            pins.push(Pin::data_out(pin_name, type_name));
        }
        pins
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let output = (self.function)(input_data);
        Ok(NodeOutput::Data(output))
    }
}

/// Blueprint Workflow
pub struct BlueprintWorkflow {
    name: String,
    entry_node: String,
    nodes: HashMap<String, Arc<dyn BlueprintNode>>,
    connections: Vec<Connection>,
}

impl BlueprintWorkflow {
    pub fn builder(name: impl Into<String>) -> BlueprintWorkflowBuilder {
        BlueprintWorkflowBuilder::new(name)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn validate(&self) -> Result<()> {
        // Check if entry node exists
        if !self.nodes.contains_key(&self.entry_node) {
            return Err(anyhow::anyhow!("Entry node not set: {}", self.entry_node).into());
        }

        // Validate all connections
        for conn in &self.connections {
            // Check if nodes exist
            let from_node = self
                .nodes
                .get(&conn.from_node)
                .ok_or_else(|| anyhow::anyhow!("Node not found: {}", conn.from_node))?;
            let to_node = self
                .nodes
                .get(&conn.to_node)
                .ok_or_else(|| anyhow::anyhow!("Node not found: {}", conn.to_node))?;

            // Check if pins exist
            let from_pins = from_node.pins();
            let to_pins = to_node.pins();

            let from_pin = from_pins
                .iter()
                .find(|p| p.name == conn.from_pin && p.direction == PinDirection::Output)
                .ok_or_else(|| {
                    anyhow::anyhow!("Output pin not found: {}.{}", conn.from_node, conn.from_pin)
                })?;

            let to_pin = to_pins
                .iter()
                .find(|p| p.name == conn.to_pin && p.direction == PinDirection::Input)
                .ok_or_else(|| {
                    anyhow::anyhow!("Input pin not found: {}.{}", conn.to_node, conn.to_pin)
                })?;

            // Check if pin types match
            match (&from_pin.pin_type, &to_pin.pin_type) {
                (PinType::Exec, PinType::Exec) => {
                    // Exec pins always match
                }
                (PinType::Data(from_type), PinType::Data(to_type)) => {
                    // Data pins must have matching type names
                    if from_type != to_type {
                        return Err(anyhow::anyhow!(
                            "Data type mismatch: {}.{} (type: {}) -> {}.{} (type: {})",
                            conn.from_node,
                            conn.from_pin,
                            from_type,
                            conn.to_node,
                            conn.to_pin,
                            to_type
                        )
                        .into());
                    }
                }
                _ => {
                    // Exec and Data pins cannot connect
                    return Err(anyhow::anyhow!(
                        "Pin category mismatch: {}.{} ({:?}) -> {}.{} ({:?})",
                        conn.from_node,
                        conn.from_pin,
                        from_pin.pin_type,
                        conn.to_node,
                        conn.to_pin,
                        to_pin.pin_type
                    )
                    .into());
                }
            }
        }

        Ok(())
    }
}

/// Blueprint Workflow Builder
pub struct BlueprintWorkflowBuilder {
    name: String,
    entry_node: Option<String>,
    nodes: HashMap<String, Arc<dyn BlueprintNode>>,
    connections: Vec<Connection>,
}

impl BlueprintWorkflowBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entry_node: None,
            nodes: HashMap::new(),
            connections: Vec::new(),
        }
    }

    pub fn entry(mut self, node_name: impl Into<String>) -> Self {
        self.entry_node = Some(node_name.into());
        self
    }

    pub fn add_node(mut self, node: Arc<dyn BlueprintNode>) -> Self {
        let name = node.name().to_string();
        self.nodes.insert(name, node);
        self
    }

    pub fn connect(
        mut self,
        from_node: impl Into<String>,
        from_pin: impl Into<String>,
        to_node: impl Into<String>,
        to_pin: impl Into<String>,
    ) -> Self {
        self.connections
            .push(Connection::new(from_node, from_pin, to_node, to_pin));
        self
    }

    pub fn build(self) -> Result<BlueprintWorkflow> {
        let entry_node = self
            .entry_node
            .ok_or_else(|| anyhow::anyhow!("Entry node not set"))?;

        let workflow = BlueprintWorkflow {
            name: self.name,
            entry_node,
            nodes: self.nodes,
            connections: self.connections,
        };

        workflow.validate()?;
        Ok(workflow)
    }
}

/// Blueprint Workflow Executor
pub struct BlueprintExecutor {
    workflow: BlueprintWorkflow,
}

impl BlueprintExecutor {
    pub fn new(workflow: BlueprintWorkflow) -> Self {
        Self { workflow }
    }

    pub async fn execute(&self, ctx: &Context) -> Result<()> {
        tracing::debug!(
            "Start executing blueprint workflow: {}",
            self.workflow.name()
        );

        let mut current_node = self.workflow.entry_node.clone();
        let mut current_pin = "exec".to_string();
        let mut data_context: HashMap<String, DataValue> = HashMap::new();

        loop {
            // Get current node
            let node = self
                .workflow
                .nodes
                .get(&current_node)
                .ok_or_else(|| anyhow::anyhow!("Node not found: {}", current_node))?;

            tracing::debug!("  Execute node: {} (pin: {})", node.name(), current_pin);

            // Execute node
            let output = node
                .execute(ctx, &current_pin, data_context.clone())
                .await?;

            // Process output
            match output {
                NodeOutput::ExecPin(next_pin) => {
                    // Find next node from connections
                    let connection = self
                        .workflow
                        .connections
                        .iter()
                        .find(|c| c.from_node == current_node && c.from_pin == next_pin)
                        .ok_or_else(|| {
                            anyhow::anyhow!("No connection found for {}.{}", current_node, next_pin)
                        })?;

                    current_node = connection.to_node.clone();
                    current_pin = connection.to_pin.clone();
                }
                NodeOutput::Data(data) => {
                    // Update data context
                    data_context.extend(data);

                    // Continue on same node if no exec pin
                    // (for data-only nodes in the middle of exec flow)
                }
                NodeOutput::Loop { .. } => {
                    // Loop is handled by executor, not used in blueprint execution
                    return Err(anyhow::anyhow!(
                        "Loop nodes are not supported in blueprint execution mode"
                    )
                    .into());
                }
                NodeOutput::Break => {
                    // Break is handled by executor, not used in blueprint execution
                    return Err(anyhow::anyhow!("Break nodes are only valid inside loops").into());
                }
                NodeOutput::Multiple(pins) => {
                    // Execute multiple branches (for Sequence node)
                    for pin in pins {
                        if let Some(connection) = self
                            .workflow
                            .connections
                            .iter()
                            .find(|c| c.from_node == current_node && c.from_pin == pin)
                        {
                            let next_node = self
                                .workflow
                                .nodes
                                .get(&connection.to_node)
                                .ok_or_else(|| {
                                    anyhow::anyhow!("Node not found: {}", connection.to_node)
                                })?;

                            next_node
                                .execute(ctx, &connection.to_pin, data_context.clone())
                                .await?;
                        }
                    }
                    tracing::debug!("  Workflow execution completed");
                    break;
                }
                NodeOutput::Complete => {
                    tracing::debug!("  Workflow execution completed");
                    break;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pin_creation() {
        let exec_in = Pin::exec_in("start");
        assert_eq!(exec_in.name, "start");
        assert_eq!(exec_in.direction, PinDirection::Input);
        assert!(exec_in.pin_type.is_exec());

        let data_out = Pin::data_out("result", "i64");
        assert_eq!(data_out.name, "result");
        assert_eq!(data_out.direction, PinDirection::Output);
        assert!(data_out.pin_type.is_data());
        assert_eq!(data_out.pin_type.type_name(), Some("i64"));
    }

    #[test]
    fn test_data_value() {
        let val = DataValue::from_i64(42);
        assert_eq!(val.type_name, "i64");
        assert_eq!(val.as_i64(), Some(42));

        let val = DataValue::from_string("hello");
        assert_eq!(val.type_name, "String");
        assert_eq!(val.as_str(), Some("hello"));
    }

    #[tokio::test]
    async fn test_entry_node() {
        let node = EntryNode::new("start");
        assert_eq!(node.name(), "start");

        let pins = node.pins();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].name, "exec");
        assert_eq!(pins[0].direction, PinDirection::Output);
    }

    #[tokio::test]
    async fn test_branch_node() {
        let node = BranchNode::new("check");
        let pins = node.pins();
        assert_eq!(pins.len(), 4);

        // Has exec input
        assert!(pins
            .iter()
            .any(|p| p.name == "exec" && p.direction == PinDirection::Input));
        // Has true/false exec outputs
        assert!(pins
            .iter()
            .any(|p| p.name == "true" && p.direction == PinDirection::Output));
        assert!(pins
            .iter()
            .any(|p| p.name == "false" && p.direction == PinDirection::Output));
        // Has condition data input
        assert!(pins
            .iter()
            .any(|p| p.name == "condition" && p.direction == PinDirection::Input));
    }
}
