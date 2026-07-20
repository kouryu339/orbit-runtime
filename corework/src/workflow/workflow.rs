//! 可复用的工作流实例
//!
//! Workflow 是工作流的运行时表示，支持：
//! - 多次执行（execute）
//! - 重置状态（reset）
//! - 查询输入输出结构（schema）

use async_recursion::async_recursion;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::error::{FrameworkError, Result};
use crate::execution_unit::{ExecutionUnit, UnitType};
use crate::workflow::core::{Connection, DataValue, NodeOutput, PinDirection, PinType};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::{traits::BlueprintNode, NodeWrapper};

/// 工作流状态在cache中的key（节点可通过ctx.get_cached/set_cached访问）
pub const WORKFLOW_STATE_KEY: &str = "__workflow_state__";

/// 工作流执行状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WorkflowState {
    /// 空闲，可以执行
    Idle,
    /// 执行中
    Running,
}

/// 引脚定义（用于描述输入输出结构）
#[derive(Debug, Clone)]
pub struct PinDefinition {
    pub name: String,
    pub data_type: String,
    pub description: String,
    pub default_value: Option<DataValue>,
}

/// 可复用的工作流实例
///
/// # 使用方式
/// ```rust,ignore
/// // 初始化全局框架（仅一次）
/// let _framework = FrameworkState::initialize()?;
///
/// // 从JSON加载工作流（自动生成唯一作用域ID！）
/// let mut workflow = Workflow::load("path/to/blueprint.json").await?;
///
/// // 第一次执行（所有缓存操作自动隔离）
/// let result1 = workflow.execute(inputs1).await?;
///
/// // 重置后再次执行（支持复用）
/// workflow.reset().await?;
/// let result2 = workflow.execute(inputs2).await?;
///
/// // 如果需要多个Workflow实例，无需担心缓存冲突
/// // 因为每个Workflow都有自己的scope_id
/// let workflow2 = Workflow::load("path/to/blueprint2.json").await?;
/// let result3 = workflow2.execute(inputs3).await?;
/// ```
pub struct Workflow {
    /// 工作流名称
    name: String,

    /// 所有节点（节点名 -> 节点实例）
    nodes: HashMap<String, Arc<NodeWrapper>>,

    /// 所有连接
    connections: Vec<Connection>,

    /// 入口节点名称（StartNode）
    entry_node: String,

    /// StartNode的输入定义（用于input_schema）
    input_pins: Vec<PinDefinition>,

    /// EndNode的输出定义（用于output_schema）
    output_pins: Vec<PinDefinition>,

    /// 默认值映射（创建时从JSON加载）
    /// 格式：cache_key -> DataValue
    default_values: HashMap<String, DataValue>,

    /// 执行单元（提供基础设施访问和资源权限管理）
    unit: Arc<ExecutionUnit>,

    /// 执行上下文（包含自动隔离的作用域缓存）
    ///
    /// 在Workflow构造时会自动生成唯一的scope_id（workflow_name_uuid），
    /// ctx.set_cached/get_cached 会自动添加scope_id前缀，
    /// 所以不同Workflow实例的缓存完全隔离，无需外部管理。
    ctx: ExecutionContext,
}

impl Workflow {
    /// 从JSON文件加载工作流（简化API）
    ///
    /// 自动从全局FrameworkState读取资源，创建具有自动隔离的Workflow实例。
    /// 每个Workflow实例都会生成唯一的scope_id，所有缓存操作自动隔离。
    ///
    /// # 示例
    /// ```rust,ignore
    /// let _framework = FrameworkState::initialize()?;
    ///
    /// let mut workflow1 = Workflow::load("blueprint1.json").await?;
    /// let mut workflow2 = Workflow::load("blueprint2.json").await?;
    ///
    /// // 虽然两个Workflow实例可能使用相同的底层cache，
    /// // 但由于scope_id不同，它们的数据完全隔离
    /// let result1 = workflow1.execute(inputs1).await?;
    /// let result2 = workflow2.execute(inputs2).await?;
    /// ```
    pub async fn load(blueprint_path: &str) -> Result<Self> {
        use crate::workflow::blueprint_loader::BlueprintLoader;
        use crate::world::FrameworkState;

        // 初始化全局框架（仅一次，OnceLock保证）
        let _framework = FrameworkState::initialize()?;

        // 从文件加载工作流
        // loader.load_workflow_from_file() 会调用 Workflow::new()
        // 在 new() 中会通过 ExecutionUnit 自动创建所有必要的资源
        let loader = BlueprintLoader::new();
        loader.load_workflow_from_file(blueprint_path).await
    }

    /// 创建新的工作流实例
    ///
    /// # 自动生成唯一ID
    /// 在构造时生成 `{workflow_name}_{uuid}` 作为作用域ID，
    /// 所有后续的缓存操作都会自动添加这个前缀进行隔离。
    /// 无需外部调用任何初始化函数。
    pub async fn new(
        name: String,
        nodes: HashMap<String, Arc<NodeWrapper>>,
        connections: Vec<Connection>,
        entry_node: String,
        input_pins: Vec<PinDefinition>,
        output_pins: Vec<PinDefinition>,
        default_values: HashMap<String, DataValue>,
    ) -> Result<Self> {
        use crate::world::FrameworkState;

        // 初始化全局框架
        let framework = FrameworkState::initialize()?;

        // 创建 ExecutionUnit
        let unit = Arc::new(ExecutionUnit::new_root(UnitType::Blueprint, framework));

        // 从 ExecutionUnit 创建 ExecutionContext
        // ExecutionContext 使用 ExecutionUnit 的 scoped_cache
        let mut ctx = ExecutionContext::new(
            unit.cache(),
            unit.event_bus(),
            unit.telemetry(),
            unit.registry(),
        );

        let variable_declarations: HashSet<String> =
            input_pins.iter().map(|pin| pin.name.clone()).collect();
        ctx.reset_workflow_variable_scope(&variable_declarations, &HashMap::new())
            .await?;

        // 初始化工作流状态到cache
        ctx.set_cached(WORKFLOW_STATE_KEY, &WorkflowState::Idle)
            .await?;

        Ok(Self {
            name,
            nodes,
            connections,
            entry_node,
            input_pins,
            output_pins,
            default_values,
            unit,
            ctx,
        })
    }

    /// 获取工作流名称
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 获取执行单元
    pub fn unit(&self) -> &Arc<ExecutionUnit> {
        &self.unit
    }

    /// 获取当前状态（从cache读取，节点也可用ctx.get_cached访问）
    pub async fn state(&self) -> WorkflowState {
        self.ctx
            .get_cached::<WorkflowState>(WORKFLOW_STATE_KEY)
            .await
            .ok()
            .flatten()
            .unwrap_or(WorkflowState::Idle)
    }

    /// 设置工作流状态（存储到cache，节点也可用ctx.set_cached修改）
    async fn set_state(&mut self, state: WorkflowState) -> Result<()> {
        self.ctx.set_cached(WORKFLOW_STATE_KEY, &state).await
    }

    /// 获取输入结构定义
    ///
    /// 返回 StartNode 的所有 DataOutput 引脚定义
    pub fn input_schema(&self) -> &[PinDefinition] {
        &self.input_pins
    }

    /// 获取输出结构定义
    ///
    /// 返回 EndNode 的所有 DataInput 引脚定义
    pub fn output_schema(&self) -> &[PinDefinition] {
        &self.output_pins
    }

    /// 执行工作流（可多次调用）
    ///
    /// # 参数
    /// - inputs: StartNode的输入参数 (pin_name -> value)
    ///
    /// # 返回
    /// - HashMap: EndNode收集的输出数据 (pin_name -> value)
    ///
    /// # 错误
    /// - 如果工作流正在运行中
    /// - 如果执行过程中发生错误
    pub async fn execute(
        &mut self,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        // 检查状态
        if self.state().await == WorkflowState::Running {
            return Err(FrameworkError::SystemError(
                "Workflow is already running".into(),
            ));
        }

        self.set_state(WorkflowState::Running).await?;

        // 1. 将inputs写入StartNode的输出引脚cache
        tracing::debug!("📥 [Workflow] 注入输入参数到 StartNode");
        self.inject_start_inputs(inputs).await?;

        // 2. 执行工作流（从StartNode的Out引脚开始）
        tracing::debug!(
            "🚀 [Workflow] 开始执行工作流，入口节点: {}",
            self.entry_node
        );
        let entry_node = self.entry_node.clone(); // 克隆避免借用冲突
        let exec_result = self.execute_from_node(&entry_node, "Out").await;

        // 3. 收集EndNode的输出
        let outputs = if exec_result.is_ok() {
            tracing::debug!("📤 [Workflow] 收集 EndNode 输出");
            self.collect_end_outputs().await?
        } else {
            HashMap::new()
        };

        self.set_state(WorkflowState::Idle).await?;

        // 4. 如果执行失败，返回错误
        exec_result.map_err(|e| {
            tracing::warn!("💥 [Workflow] 工作流执行失败: {}", e);
            e
        })?;

        tracing::debug!("✅ [Workflow] 工作流执行成功完成");
        Ok(outputs)
    }

    // ==================== 查询方法 ====================
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// 获取连接数量
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// 获取入口节点名称
    pub fn entry_node(&self) -> &str {
        &self.entry_node
    }

    // ==================== 重置方法 ====================

    /// 重置工作流到初始状态
    ///
    /// 清空所有缓存并恢复默认值
    pub async fn reset(&mut self) -> Result<()> {
        tracing::debug!("🔄 [Workflow] 重置工作流状态");

        // Future: 需要在 ExecutionContext 中添加 clear_all() 方法
        // self.ctx.clear_all().await?;

        // 恢复所有默认值
        for (cache_key, value) in &self.default_values {
            self.ctx.set_cached(cache_key, value).await?;
        }

        self.set_state(WorkflowState::Idle).await?;
        tracing::debug!("✅ [Workflow] 重置完成");
        Ok(())
    }

    // ==================== 内部执行方法 ====================

    /// 将输入参数注入到StartNode的输出引脚cache
    async fn inject_start_inputs(&mut self, inputs: HashMap<String, DataValue>) -> Result<()> {
        for (pin_name, value) in inputs {
            let cache_key = format!("{}:{}", self.entry_node, pin_name);
            tracing::debug!("  📌 设置 {} = {:?}", cache_key, value);
            self.ctx.set_cached(&cache_key, &value).await?;
            self.ctx.set_workflow_variable(&pin_name, &value).await?;
        }
        Ok(())
    }

    /// 收集EndNode的输出数据
    async fn collect_end_outputs(&self) -> Result<HashMap<String, DataValue>> {
        let mut results = HashMap::new();

        // 查找所有 EndNode
        for node in self.nodes.values() {
            if node.as_ref().name() == "End" {
                // 获取 EndNode 的输入映射
                let (input_mappings, _) = node.as_ref().pin_cache_mappings();

                // 收集其输入数据作为结果
                for mapping in input_mappings {
                    if let Ok(Some(value)) =
                        self.ctx.get_cached::<DataValue>(&mapping.cache_key).await
                    {
                        results.insert(mapping.pin_name.clone(), value);
                    }
                }
            }
        }

        Ok(results)
    }

    /// 从指定节点的指定输出引脚开始执行
    #[async_recursion]
    async fn execute_from_node(&mut self, node_name: &str, exec_pin: &str) -> Result<()> {
        // 防止无限循环
        if self.ctx.flow().is_max_steps_reached() {
            return Err(FrameworkError::SystemError(
                "Max execution steps reached".into(),
            ));
        }

        // 移动到当前节点
        self.ctx
            .flow_mut()
            .move_to(node_name.to_string(), exec_pin.to_string());

        // 查找从当前节点的执行引脚连接出去的下一个节点
        let next_connections: Vec<String> = self
            .connections
            .iter()
            .filter(|conn| conn.from_node == node_name && conn.from_pin == exec_pin)
            .map(|conn| conn.to_node.clone()) // 克隆to_node避免借用冲突
            .collect();

        for to_node in next_connections {
            // 执行目标节点 - 如果执行失败，立即停止并返回错误
            self.execute_node(&to_node).await.inspect_err(|_e| {
                tracing::warn!(
                    "❌ 工作流执行停止: 从节点 '{}' 的 '{}' 引脚执行到节点 '{}' 时失败",
                    node_name,
                    exec_pin,
                    to_node
                );
            })?;
        }

        Ok(())
    }

    /// 执行单个节点
    #[async_recursion]
    async fn execute_node(&mut self, node_name: &str) -> Result<()> {
        let node = self
            .nodes
            .get(node_name)
            .ok_or_else(|| FrameworkError::SystemError(format!("Node not found: {}", node_name)))?
            .clone(); // 克隆Arc避免借用冲突

        tracing::debug!(
            "🔧 [Workflow] 执行节点: {} (内部name: '{}', type: {:?})",
            node_name,
            node.as_ref().name(),
            node.as_ref().node_type()
        );

        // 收集输入数据
        let inputs = self.collect_inputs(node_name).await.map_err(|e| {
            FrameworkError::SystemError(format!("节点 '{}' 收集输入失败: {}", node_name, e))
        })?;
        tracing::debug!("   📥 [Workflow] 收集到 {} 个输入", inputs.len());

        // 执行节点
        tracing::debug!("   ⚡ [Workflow] 调用 node.execute_node()...");
        let output = node
            .as_ref()
            .execute_node(&mut self.ctx, inputs)
            .await
            .map_err(|e| {
                tracing::warn!(
                    "❌ 错误: 节点 '{}' (类型: '{}') 执行失败: {}",
                    node_name,
                    node.as_ref().name(),
                    e
                );
                FrameworkError::SystemError(format!(
                    "节点 '{}' (类型: '{}') 执行失败: {}",
                    node_name,
                    node.as_ref().name(),
                    e
                ))
            })?;
        tracing::debug!("   ✅ [Workflow] execute_node 返回: {:?}", output);

        // 根据输出决定下一步执行
        match output {
            NodeOutput::ExecPin(pin_name) => {
                // 执行单个输出引脚
                self.execute_from_node(node_name, &pin_name).await?;
            }
            NodeOutput::Multiple(pin_names) => {
                // 顺序执行多个输出引脚
                for pin_name in pin_names {
                    self.execute_from_node(node_name, &pin_name).await?;
                }
            }
            NodeOutput::Data(outputs) => {
                // 存储输出数据，然后继续执行默认输出引脚
                self.store_outputs(node_name, outputs).await?;
                self.execute_from_node(node_name, "Then").await?;
            }
            NodeOutput::Loop {
                body_pin,
                completed_pin,
                iterations,
            } => {
                // 循环执行（旧版 workflow，不支持 Break）
                for iteration in iterations {
                    self.store_outputs(node_name, iteration.outputs).await?;
                    self.execute_from_node(node_name, &body_pin).await?;
                }
                self.execute_from_node(node_name, &completed_pin).await?;
            }
            NodeOutput::Break => {
                // Break 应该使用新的 BlueprintExecutor，此处不支持
                return Err(FrameworkError::SystemError(
                    "Break 节点只在使用 BlueprintExecutor 时有效".into(),
                ));
            }
            NodeOutput::Complete => {
                // 工作流完成
                return Ok(());
            }
        }

        Ok(())
    }

    /// 收集节点的输入数据
    async fn collect_inputs(&self, node_name: &str) -> Result<HashMap<String, DataValue>> {
        let node = self
            .nodes
            .get(node_name)
            .ok_or_else(|| FrameworkError::SystemError(format!("Node not found: {}", node_name)))?;

        let mut inputs = HashMap::new();

        // 遍历节点的所有输入引脚
        for pin in node.as_ref().pins() {
            if pin.direction == PinDirection::Input {
                // 跳过执行引脚
                if matches!(pin.pin_type, PinType::Exec) {
                    continue;
                }

                // 查找连接到此引脚的数据
                if let Some(conn) = self
                    .connections
                    .iter()
                    .find(|c| c.to_node == node_name && c.to_pin == pin.name)
                {
                    // 查找源节点的输出映射
                    let source_node = self.nodes.get(&conn.from_node).ok_or_else(|| {
                        FrameworkError::SystemError(format!(
                            "Source node not found: {}",
                            conn.from_node
                        ))
                    })?;
                    let (_, source_output_mappings) = source_node.as_ref().pin_cache_mappings();

                    // 查找cache key：优先使用映射，否则使用旧格式
                    let cache_key = if let Some(mapping) = source_output_mappings
                        .iter()
                        .find(|m| m.pin_name == conn.from_pin)
                    {
                        mapping.cache_key.clone()
                    } else {
                        format!("{}:{}", conn.from_node, conn.from_pin)
                    };

                    if let Ok(Some(value)) = self.ctx.get_cached::<DataValue>(&cache_key).await {
                        inputs.insert(pin.name.clone(), value);
                    }
                } else {
                    // 引脚未连接，尝试从节点的默认值获取
                    let default_key = format!("{}:defaults:{}", node_name, pin.name);
                    if let Ok(Some(value)) = self.ctx.get_cached::<DataValue>(&default_key).await {
                        inputs.insert(pin.name.clone(), value);
                    }
                }
            }
        }

        Ok(inputs)
    }

    /// 存储节点的输出数据到上下文
    async fn store_outputs(
        &self,
        node_name: &str,
        outputs: HashMap<String, DataValue>,
    ) -> Result<()> {
        let node = self
            .nodes
            .get(node_name)
            .ok_or_else(|| FrameworkError::SystemError(format!("Node not found: {}", node_name)))?;
        let (_, output_mappings) = node.as_ref().pin_cache_mappings();

        for (pin_name, value) in outputs {
            // 查找cache key：优先使用映射，否则使用旧格式
            let cache_key =
                if let Some(mapping) = output_mappings.iter().find(|m| m.pin_name == pin_name) {
                    mapping.cache_key.clone()
                } else {
                    format!("{}:{}", node_name, pin_name)
                };
            self.ctx.set_cached(&cache_key, &value).await?;
        }
        Ok(())
    }
}
