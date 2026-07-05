//! Blueprint 执行器 - 核心执行引擎
//!
//! 负责遍历蓝图图并执行节点

use async_recursion::async_recursion;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{FrameworkError, Result};
use crate::workflow::core::{Connection, DataValue, NodeOutput, PinDirection, PinType};
use crate::workflow::execution::ExecutionContext;
use crate::workflow::nodes::{BlueprintNode, NodeType, NodeWrapper};

/// 循环控制信号
enum LoopControl {
    Continue, // 继续循环
    Break,    // 中断循环
}

/// 蓝图执行器
#[derive(Debug, Clone)]
pub struct BlueprintExecutor {
    /// 所有节点（节点名 -> 节点实例）
    nodes: HashMap<String, Arc<NodeWrapper>>,
    /// 所有连接
    connections: Vec<Connection>,
    /// 入口节点名称
    entry_node: Option<String>,
}

impl BlueprintExecutor {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            connections: Vec::new(),
            entry_node: None,
        }
    }

    /// 添加节点（使用外部提供的key）
    pub fn add_node(&mut self, key: String, node: Arc<NodeWrapper>) {
        self.nodes.insert(key, node);
    }

    /// 添加连接
    pub fn add_connection(&mut self, conn: Connection) {
        self.connections.push(conn);
    }

    /// 设置入口节点
    pub fn set_entry(&mut self, node_name: impl Into<String>) {
        self.entry_node = Some(node_name.into());
    }

    /// 执行蓝图，支持输入参数和返回值
    ///
    /// # 参数
    /// - ctx: 执行上下文
    /// - inputs: StartNode的输入参数 (pin_name -> value)
    ///
    /// # 返回
    /// - HashMap: EndNode收集的输出数据 (pin_name -> value)
    pub async fn execute_with_params(
        &self,
        ctx: &mut ExecutionContext,
        inputs: HashMap<String, DataValue>,
    ) -> Result<HashMap<String, DataValue>> {
        // 找到入口节点
        let entry_name = self
            .entry_node
            .as_ref()
            .ok_or_else(|| FrameworkError::SystemError("No entry node defined".into()))?;

        // 初始化 StartNode 的输入参数到 cache
        // StartNode 的输出格式为 {node_name}:{pin_name}
        for (pin_name, value) in inputs {
            let cache_key = format!("{}:{}", entry_name, pin_name);
            ctx.set_cached(&cache_key, &value).await?;
            ctx.set_workflow_variable(&pin_name, &value).await?;
        }

        // 从入口开始执行
        tracing::debug!("🚀 [Executor] 开始执行工作流，入口节点: {}", entry_name);

        // StartNode returns ExecPin("Out"), and execute_node() already follows it.
        // Calling execute_from_node("Out") here would run the whole chain twice.
        self.execute_node(ctx, entry_name).await?;
        tracing::debug!("✅ [Executor] 工作流执行成功完成");

        // 收集 EndNode 的输出结果
        // EndNode 使用的 cache key 格式为 "end::result" 或自定义格式
        let mut results = HashMap::new();

        // 查找所有 EndNode
        for node in self.nodes.values() {
            if node.name() == "End" {
                // 获取 EndNode 的输入映射
                let (input_mappings, _) = node.pin_cache_mappings();

                // 收集其输入数据作为结果
                for mapping in input_mappings {
                    if let Ok(Some(value)) = ctx.get_cached::<DataValue>(&mapping.cache_key).await {
                        results.insert(mapping.pin_name.clone(), value);
                    }
                }
            }
        }

        Ok(results)
    }

    /// 执行蓝图（向后兼容的简化版本）
    pub async fn execute(&self, ctx: &mut ExecutionContext) -> Result<()> {
        // 找到入口节点
        let entry_name = self
            .entry_node
            .as_ref()
            .ok_or_else(|| FrameworkError::SystemError("No entry node defined".into()))?;

        // 从入口开始执行
        tracing::debug!("🚀 [Executor] 开始执行工作流，入口节点: {}", entry_name);
        self.execute_from_node(ctx, entry_name, "Out")
            .await
            .map_err(|e| {
                tracing::warn!("💥 [Executor] 工作流执行失败: {}", e);
                e
            })?;
        tracing::debug!("✅ [Executor] 工作流执行成功完成");

        Ok(())
    }

    /// 从指定节点的指定输出引脚开始执行（带 Break 检测）
    #[async_recursion]
    async fn execute_from_node_with_break(
        &self,
        ctx: &mut ExecutionContext,
        node_name: &str,
        exec_pin: &str,
    ) -> Result<LoopControl> {
        // 防止无限循环
        if ctx.flow().is_max_steps_reached() {
            return Err(FrameworkError::SystemError(
                "Max execution steps reached".into(),
            ));
        }

        // 移动到当前节点
        ctx.flow_mut()
            .move_to(node_name.to_string(), exec_pin.to_string());

        // 查找从当前节点的执行引脚连接出去的下一个节点
        let next_connections: Vec<_> = self
            .connections
            .iter()
            .filter(|conn| conn.from_node == node_name && conn.from_pin == exec_pin)
            .collect();

        for conn in next_connections {
            // Execute target node through the same trace-aware path used by normal flow.
            let output = self.execute_single_node(ctx, &conn.to_node).await?;

            // 检查是否是 Break 信号（execute_from_node_with_break）
            match output {
                NodeOutput::Break => {
                    return Ok(LoopControl::Break);
                }
                NodeOutput::ExecPin(pin_name) => {
                    // 递归执行下一个节点，继续检查 Break
                    let control = self
                        .execute_from_node_with_break(ctx, &conn.to_node, &pin_name)
                        .await?;
                    if matches!(control, LoopControl::Break) {
                        return Ok(LoopControl::Break);
                    }
                }
                NodeOutput::Multiple(pin_names) => {
                    for pin_name in pin_names {
                        let control = self
                            .execute_from_node_with_break(ctx, &conn.to_node, &pin_name)
                            .await?;
                        if matches!(control, LoopControl::Break) {
                            return Ok(LoopControl::Break);
                        }
                    }
                }
                NodeOutput::Data(outputs) => {
                    self.store_outputs(ctx, &conn.to_node, outputs).await?;
                    let control = self
                        .execute_from_node_with_break(ctx, &conn.to_node, "Then")
                        .await?;
                    if matches!(control, LoopControl::Break) {
                        return Ok(LoopControl::Break);
                    }
                }
                NodeOutput::Complete => {
                    return Ok(LoopControl::Continue);
                }
                NodeOutput::Loop { .. } => {
                    // 嵌套循环：递归执行，内层 Break 不影响外层
                    self.execute_node(ctx, &conn.to_node).await?;
                }
            }
        }

        Ok(LoopControl::Continue)
    }

    /// 从指定节点的指定输出引脚开始执行
    #[async_recursion]
    async fn execute_from_node(
        &self,
        ctx: &mut ExecutionContext,
        node_name: &str,
        exec_pin: &str,
    ) -> Result<()> {
        // 防止无限循环
        if ctx.flow().is_max_steps_reached() {
            return Err(FrameworkError::SystemError(
                "Max execution steps reached".into(),
            ));
        }

        // 移动到当前节点
        ctx.flow_mut()
            .move_to(node_name.to_string(), exec_pin.to_string());

        // 查找从当前节点的执行引脚连接出去的下一个节点
        let next_connections: Vec<_> = self
            .connections
            .iter()
            .filter(|conn| conn.from_node == node_name && conn.from_pin == exec_pin)
            .collect();

        for conn in next_connections {
            // 执行目标节点 - 如果执行失败，立即停止并返回错误
            self.execute_node(ctx, &conn.to_node)
                .await
                .inspect_err(|_e| {
                    tracing::warn!(
                        "❌ 工作流执行停止: 从节点 '{}' 的 '{}' 引脚执行到节点 '{}' 时失败",
                        node_name,
                        exec_pin,
                        conn.to_node
                    );
                })?;
        }

        Ok(())
    }

    async fn execute_single_node(
        &self,
        ctx: &mut ExecutionContext,
        node_name: &str,
    ) -> Result<NodeOutput> {
        let node =
            self.nodes.get(node_name).cloned().ok_or_else(|| {
                FrameworkError::SystemError(format!("Node not found: {}", node_name))
            })?;

        tracing::debug!(
            "🔧 [Executor] 执行节点: {} (内部name: '{}', type: {:?})",
            node_name,
            node.name(),
            node.node_type()
        );
        tracing::debug!(
            "Executing node: {} (name: '{}', type: {:?})",
            node_name,
            node.name(),
            node.node_type()
        );

        ctx.trace_begin_node(node_name.to_string(), format!("{:?}", node.node_type()));

        let inputs = match self.collect_inputs(ctx, node_name).await {
            Ok(inputs) => inputs,
            Err(e) => {
                ctx.trace_fail_node(node_name, e.to_string());
                return Err(FrameworkError::SystemError(format!(
                    "节点 '{}' 收集输入失败: {}",
                    node_name, e
                )));
            }
        };
        tracing::debug!("   📥 [Executor] 收集到 {} 个输入", inputs.len());

        tracing::debug!("   ⚡ [Executor] 调用 node.execute_node()...");
        let output = match node.execute_node(ctx, inputs).await {
            Ok(output) => output,
            Err(e) => {
                ctx.trace_fail_node(node_name, e.to_string());
                tracing::warn!(
                    "❌ 错误: 节点 '{}' (类型: '{}') 执行失败: {}",
                    node_name,
                    node.name(),
                    e
                );
                return Err(FrameworkError::SystemError(format!(
                    "节点 '{}' (类型: '{}') 执行失败: {}",
                    node_name,
                    node.name(),
                    e
                )));
            }
        };
        tracing::debug!("   ✅ [Executor] execute_node 返回: {:?}", output);

        let output_pin = match &output {
            NodeOutput::ExecPin(pin_name) => Some(pin_name.clone()),
            NodeOutput::Multiple(pin_names) => Some(pin_names.join(",")),
            NodeOutput::Data(_) => Some("Then".to_string()),
            NodeOutput::Loop { completed_pin, .. } => Some(completed_pin.clone()),
            NodeOutput::Break => Some("Break".to_string()),
            NodeOutput::Complete => Some("Complete".to_string()),
        };
        ctx.trace_finish_node(node_name, output_pin);

        Ok(output)
    }

    /// 执行单个节点
    #[async_recursion]
    async fn execute_node(&self, ctx: &mut ExecutionContext, node_name: &str) -> Result<()> {
        let output = self.execute_single_node(ctx, node_name).await?;

        // 根据输出决定下一步执行
        match output {
            NodeOutput::ExecPin(pin_name) => {
                // 执行单个输出引脚
                self.execute_from_node(ctx, node_name, &pin_name).await?;
            }
            NodeOutput::Multiple(pin_names) => {
                // 顺序执行多个输出引脚
                for pin_name in pin_names {
                    self.execute_from_node(ctx, node_name, &pin_name).await?;
                }
            }
            NodeOutput::Data(outputs) => {
                // 存储输出数据，然后继续执行默认输出引脚
                self.store_outputs(ctx, node_name, outputs).await?;
                self.execute_from_node(ctx, node_name, "Then").await?;
            }
            NodeOutput::Loop {
                body_pin,
                completed_pin,
                iterations,
            } => {
                tracing::debug!(
                    "   🔁 [Executor] 开始循环执行，共 {} 次迭代",
                    iterations.len()
                );

                let mut loop_broke = false;

                // 遍历所有迭代
                for (index, iteration) in iterations.iter().enumerate() {
                    tracing::debug!(
                        "   🔄 [Executor] 执行迭代 {}/{}",
                        index + 1,
                        iterations.len()
                    );

                    // 1. 存储当前迭代的输出数据（如 Index）
                    self.store_outputs(ctx, node_name, iteration.outputs.clone())
                        .await?;

                    // 2. 执行循环体分支，检查是否有 Break
                    let control = self
                        .execute_from_node_with_break(ctx, node_name, &body_pin)
                        .await?;

                    if matches!(control, LoopControl::Break) {
                        tracing::debug!("   🛑 [Executor] 捕获到 Break 信号，提前退出循环");
                        loop_broke = true;
                        break;
                    }

                    // 3. 检查是否超过最大步数
                    if ctx.flow().is_max_steps_reached() {
                        return Err(FrameworkError::SystemError(
                            "Max execution steps reached in loop".into(),
                        ));
                    }
                }

                if loop_broke {
                    tracing::debug!(
                        "   🛑 [Executor] Break 后执行 Completed 分支（提前退出但仍收集结果）"
                    );
                } else {
                    tracing::debug!("   ✅ [Executor] 循环正常完成，执行 Completed 分支");
                }
                // Break 和正常结束都执行 Completed（与 UE5 Blueprint 行为一致）
                self.execute_from_node(ctx, node_name, &completed_pin)
                    .await?;
            }
            NodeOutput::Break => {
                // Break 应该在循环体内部被捕获，如果到达这里说明有问题
                return Err(FrameworkError::SystemError(
                    "Break 节点只能在循环内部使用".into(),
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
    ///
    /// Pure 节点每次都强制重算（无惰性求值），保证循环迭代中上游值变化能正确传播。
    #[async_recursion]
    async fn collect_inputs(
        &self,
        ctx: &mut ExecutionContext,
        node_name: &str,
    ) -> Result<HashMap<String, DataValue>> {
        // ── 第一遍（同步）：收集需要重算的 Pure 源节点 ──────────────────────
        // Pure 节点无惰性求值：每次调用都强制重算，保证循环中上游值变化正确传播。
        // 将 self.nodes 的借用限定在同步块内，不跨越 .await 点。
        let pure_sources_to_eval: Vec<String> = {
            let node = self.nodes.get(node_name).ok_or_else(|| {
                FrameworkError::SystemError(format!("Node not found: {}", node_name))
            })?;

            let mut sources = Vec::new();
            for pin in node.pins() {
                if pin.direction != PinDirection::Input || matches!(pin.pin_type, PinType::Exec) {
                    continue;
                }
                if let Some(conn) = self
                    .connections
                    .iter()
                    .find(|c| c.to_node == node_name && c.to_pin == pin.name)
                {
                    if let Some(src) = self.nodes.get(&conn.from_node) {
                        if matches!(src.node_type(), NodeType::Pure) {
                            sources.push(conn.from_node.clone());
                        }
                    }
                }
            }
            sources
        }; // self.nodes 借用在此释放

        // ── 强制重算上游 Pure 节点（递归传导，覆盖整条 Pure 链）────────────
        // Pure 节点无惰性求值：每次父节点（无论 Pure 还是 Impure）执行都重算。
        // 缓存仍然写入，供同一次执行里多处消费读取，但不作为"跳过"依据。
        for src_name in pure_sources_to_eval {
            tracing::debug!("[Executor] 强制重算 Pure 节点: {}", src_name);
            self.execute_node(ctx, &src_name).await?;
        }

        // ── 第二遍：收集所有输入 ─────────────────────────────────────────────
        // 同步地构建 (pin_name, cache_key) 列表，不持有 .await
        let pin_cache_lookups: Vec<(String, String)> = {
            let node = self.nodes.get(node_name).ok_or_else(|| {
                FrameworkError::SystemError(format!("Node not found: {}", node_name))
            })?;

            let mut lookups = Vec::new();
            let (input_mappings, _) = node.pin_cache_mappings();
            let _ = input_mappings;

            for pin in node.pins() {
                if pin.direction != PinDirection::Input || matches!(pin.pin_type, PinType::Exec) {
                    continue;
                }
                let pin_name = pin.name.clone();
                if let Some(conn) = self
                    .connections
                    .iter()
                    .find(|c| c.to_node == node_name && c.to_pin == pin_name)
                {
                    let source_node = self.nodes.get(&conn.from_node).ok_or_else(|| {
                        FrameworkError::SystemError(format!(
                            "Source node not found: {}",
                            conn.from_node
                        ))
                    })?;
                    let (_, source_output_mappings) = source_node.pin_cache_mappings();
                    let cache_key = source_output_mappings
                        .iter()
                        .find(|m| m.pin_name == conn.from_pin)
                        .map(|m| m.cache_key.clone())
                        .unwrap_or_else(|| format!("{}:{}", conn.from_node, conn.from_pin));
                    lookups.push((pin_name, cache_key));
                } else {
                    // 未连接 → 用节点默认值
                    lookups.push((
                        pin_name.clone(),
                        format!("{}:defaults:{}", node_name, pin_name),
                    ));
                }
            }
            lookups
        }; // self.nodes 借用全部在此释放

        // 异步：批量查找 cache
        let mut inputs = HashMap::new();
        for (pin_name, cache_key) in pin_cache_lookups {
            if let Ok(Some(value)) = ctx.get_cached::<DataValue>(&cache_key).await {
                inputs.insert(pin_name, value);
            } else {
                tracing::debug!(
                    "Cache miss (or not connected) for {}.{} → key={}",
                    node_name,
                    pin_name,
                    cache_key
                );
            }
        }

        Ok(inputs)
    }

    /// 存储节点的输出数据到上下文
    async fn store_outputs(
        &self,
        ctx: &ExecutionContext,
        node_name: &str,
        outputs: HashMap<String, DataValue>,
    ) -> Result<()> {
        let node = self
            .nodes
            .get(node_name)
            .ok_or_else(|| FrameworkError::SystemError(format!("Node not found: {}", node_name)))?;
        let (_, output_mappings) = node.pin_cache_mappings();

        for (pin_name, value) in outputs {
            // 查找cache key：优先使用映射，否则使用旧格式
            let cache_key =
                if let Some(mapping) = output_mappings.iter().find(|m| m.pin_name == pin_name) {
                    mapping.cache_key.clone()
                } else {
                    format!("{}:{}", node_name, pin_name)
                };
            ctx.set_cached(&cache_key, &value).await?;
        }
        Ok(())
    }
}

impl Default for BlueprintExecutor {
    fn default() -> Self {
        Self::new()
    }
}
