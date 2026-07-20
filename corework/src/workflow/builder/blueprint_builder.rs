//! Blueprint Builder - 流畅的蓝图构建 API
//!
//! 提供开箱即用的蓝图构建体验

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::error::{FrameworkError, Result};
use crate::workflow::compiler::BlueprintCompiler;
use crate::workflow::core::{Connection, DataValue};
use crate::workflow::execution::{BlueprintExecutor, WorkflowSourceRef};
use crate::workflow::nodes::control::PinMapping;
use crate::workflow::nodes::{BlueprintNode, NodeWrapper};

/// Blueprint Builder - 链式 API
pub struct BlueprintBuilder {
    name: String,
    nodes: Vec<(String, Arc<NodeWrapper>)>, // (key, node)
    connections: Vec<Connection>,
    entry_node: Option<String>,
    auto_validate: bool,
}

impl BlueprintBuilder {
    /// 创建新的 Builder
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            nodes: Vec::new(),
            connections: Vec::new(),
            entry_node: None,
            auto_validate: true,
        }
    }

    /// 添加节点
    pub fn add_node_with_key(mut self, key: String, node: Arc<NodeWrapper>) -> Self {
        self.nodes.push((key, node));
        self
    }

    /// 添加节点（自动使用节点name作为key）
    pub fn add_node(mut self, node: Arc<NodeWrapper>) -> Self {
        let key = node.name().to_string();
        self.nodes.push((key, node));
        self
    }

    /// 快速添加 Add 节点
    pub fn add_add(self, name: impl Into<String>) -> Self {
        let name_str = name.into();
        self.add_node_with_key(name_str.clone(), NodeWrapper::add(name_str))
    }

    /// 快速添加 Multiply 节点
    pub fn add_multiply(self, name: impl Into<String>) -> Self {
        let name_str = name.into();
        self.add_node_with_key(name_str.clone(), NodeWrapper::multiply(name_str))
    }

    /// 快速添加 Greater 节点
    pub fn add_greater(self, name: impl Into<String>) -> Self {
        let name_str = name.into();
        self.add_node_with_key(name_str.clone(), NodeWrapper::greater(name_str))
    }

    /// 快速添加 Branch 节点
    pub fn add_branch(self, name: impl Into<String>) -> Self {
        let name_str = name.into();
        self.add_node_with_key(name_str.clone(), NodeWrapper::branch(name_str))
    }

    /// 快速添加 Start 节点（执行入口）
    pub fn add_start(mut self, name: impl Into<String>) -> Self {
        let name_str = name.into();
        if self.entry_node.is_none() {
            self.entry_node = Some(name_str.clone());
        }
        self.add_node_with_key(name_str.clone(), NodeWrapper::start(name_str))
    }

    /// 快速添加 Start 节点（带多个输出引脚映射）
    pub fn add_start_with_outputs(
        mut self,
        name: impl Into<String>,
        outputs: Vec<PinMapping>,
    ) -> Self {
        let name_str = name.into();
        if self.entry_node.is_none() {
            self.entry_node = Some(name_str.clone());
        }
        self.add_node_with_key(
            name_str.clone(),
            NodeWrapper::start_with_outputs(name_str, outputs),
        )
    }

    /// 快速添加 End 节点（执行终止）
    pub fn add_end(self, name: impl Into<String>) -> Self {
        let name_str = name.into();
        self.add_node_with_key(name_str.clone(), NodeWrapper::end(name_str))
    }

    /// 快速添加 End 节点（自定义结果缓存键）
    pub fn add_end_with_cache_key(
        self,
        name: impl Into<String>,
        cache_key: impl Into<String>,
        result_pin: impl Into<String>,
        result_type: impl Into<String>,
    ) -> Self {
        let name_str = name.into();
        self.add_node_with_key(
            name_str.clone(),
            NodeWrapper::end_with_cache_key(name_str, cache_key, result_pin, result_type),
        )
    }

    /// 快速添加 End 节点（带多个输入引脚映射）
    pub fn add_end_with_inputs(self, name: impl Into<String>, inputs: Vec<PinMapping>) -> Self {
        let name_str = name.into();
        self.add_node_with_key(
            name_str.clone(),
            NodeWrapper::end_with_inputs(name_str, inputs),
        )
    }

    /// 添加任意节点（BlueprintNode）
    ///
    /// 这允许业务层扩展自定义节点：
    /// ```rust,ignore
    /// let custom_node = MyCustomNode::new();
    /// let builder = BlueprintBuilder::new("Example")
    ///     .add_impure_node("my_custom_node", custom_node);
    /// ```
    pub fn add_impure_node<N: BlueprintNode + Send + Sync + 'static>(
        self,
        key: impl Into<String>,
        node: N,
    ) -> Self {
        self.add_node_with_key(key.into(), NodeWrapper::from_blueprint(node))
    }

    /// 添加动态节点（来自 NodeRegistry）
    pub fn add_dynamic_node(
        self,
        key: impl Into<String>,
        node: std::sync::Arc<dyn crate::workflow::nodes::traits::BlueprintNode + Send + Sync>,
    ) -> Self {
        self.add_node_with_key(key.into(), Arc::new(NodeWrapper::DynamicBlueprint(node)))
    }

    /// 连接两个节点
    pub fn connect(
        mut self,
        from_node: impl Into<String>,
        from_pin: impl Into<String>,
        to_node: impl Into<String>,
        to_pin: impl Into<String>,
    ) -> Self {
        self.connections.push(Connection {
            from_node: from_node.into(),
            from_pin: from_pin.into(),
            to_node: to_node.into(),
            to_pin: to_pin.into(),
        });
        self
    }

    /// 快速连接数据引脚（自动使用 Result 和 A/B 引脚）
    pub fn connect_data(
        self,
        from_node: impl Into<String>,
        to_node: impl Into<String>,
        to_pin: impl Into<String>,
    ) -> Self {
        self.connect(from_node, "Result", to_node, to_pin)
    }

    /// 快速连接执行引脚
    pub fn connect_exec(
        self,
        from_node: impl Into<String>,
        from_pin: impl Into<String>,
        to_node: impl Into<String>,
    ) -> Self {
        self.connect(from_node, from_pin, to_node, "In")
    }

    /// 设置入口节点
    pub fn set_entry(mut self, node_name: impl Into<String>) -> Self {
        self.entry_node = Some(node_name.into());
        self
    }

    /// 禁用自动验证
    pub fn skip_validation(mut self) -> Self {
        self.auto_validate = false;
        self
    }

    /// 编译蓝图（验证 + 构建）
    pub fn compile(self) -> Result<CompiledBlueprint> {
        // Future: 添加验证
        // let report = if self.auto_validate {
        //     self.validate()?
        // } else {
        //     ValidationReport::default()
        // };

        // 构建执行器
        let mut executor = BlueprintExecutor::new();

        // 添加所有节点
        for (key, node) in &self.nodes {
            executor.add_node(key.clone(), node.clone());
        }

        // 添加所有连接
        for conn in &self.connections {
            executor.add_connection(conn.clone());
        }

        // 设置入口节点
        if let Some(entry) = &self.entry_node {
            executor.set_entry(entry);
        }

        Ok(CompiledBlueprint {
            name: self.name,
            executor,
            node_defaults: HashMap::new(), // 初始化为空，将由BlueprintLoader填充
            variable_declarations: HashSet::new(),
            variable_defaults: HashMap::new(),
            source_map: HashMap::new(),
            // validation_report: report,
        })
    }

    // Future: 重新实现验证
    /*
    /// 只验证不构建
    pub fn validate(&self) -> Result<ValidationReport> {
        // 1. 图结构验证
        let node_names: HashSet<String> = self.nodes.iter()
            .map(|n| n.name().to_string())
            .collect();

        let graph_validator = GraphValidator::new(node_names, self.connections.clone());
        let mut report = graph_validator.validate()?;

        // 2. 类型检查
        let mut type_checker = TypeChecker::new();
        for node in &self.nodes {
            type_checker.register_node(&**node);
        }

        let type_warnings = type_checker.check_connections(&self.connections)?;
        report.warnings.extend(type_warnings);

        Ok(report)
    }
    */

    /// 构建（不验证）
    pub fn build_unchecked(self) -> BlueprintExecutor {
        let mut executor = BlueprintExecutor::new();

        for (key, node) in self.nodes {
            executor.add_node(key, node);
        }

        for conn in self.connections {
            executor.add_connection(conn);
        }

        if let Some(entry) = self.entry_node {
            executor.set_entry(entry);
        }

        executor
    }

    /// 构建为可复用的Workflow实例
    ///
    /// # 编译期检查
    /// 1. 验证所有连接的节点都存在
    /// 2. **数据流环检测**（DAG验证）
    /// 3. Future: 验证引脚类型匹配
    /// 4. Future: 类型推导
    ///
    /// # 使用方式
    /// ```rust,ignore
    /// let workflow = BlueprintBuilder::new("MyWorkflow")
    ///     .add_start("start")
    ///     .add_end("end")
    ///     .connect_exec("start", "Out", "end", "In")
    ///     .build(ctx)
    ///     .await?;
    ///
    /// // 可以多次执行
    /// let result1 = workflow.execute(inputs1).await?;
    /// workflow.reset().await?;
    /// let result2 = workflow.execute(inputs2).await?;
    /// ```
    pub async fn build(self) -> Result<crate::workflow::workflow::Workflow> {
        use crate::workflow::workflow::Workflow;

        // 1. 收集所有节点名称
        let node_names: Vec<String> = self.nodes.iter().map(|(key, _)| key.clone()).collect();

        // 2. 验证连接的节点是否存在
        for conn in &self.connections {
            if !node_names.contains(&conn.from_node) {
                return Err(FrameworkError::SystemError(format!(
                    "连接引用了不存在的节点: {}",
                    conn.from_node
                )));
            }
            if !node_names.contains(&conn.to_node) {
                return Err(FrameworkError::SystemError(format!(
                    "连接引用了不存在的节点: {}",
                    conn.to_node
                )));
            }
        }

        // 3. ✨ 数据流环检测（核心安全检查）
        tracing::debug!("🔍 [Compiler] 执行数据流环检测...");
        BlueprintCompiler::detect_data_cycles(&node_names, &self.connections)?;
        tracing::debug!("✅ [Compiler] 环检测通过，数据流为DAG");

        // 4. 验证入口节点
        let entry_node = self.entry_node.ok_or_else(|| {
            FrameworkError::SystemError(
                "No entry node set. Use add_start() to set an entry node.".to_string(),
            )
        })?;

        // 5. 构建节点映射
        let mut nodes = HashMap::new();
        for (key, node) in self.nodes {
            nodes.insert(key, node);
        }

        // Future: 从节点中提取input/output pins和默认值
        // 当前简化实现，后续可以根据StartNode和EndNode的配置自动提取
        let input_pins = Vec::new();
        let output_pins = Vec::new();
        let default_values = HashMap::new();

        Workflow::new(
            self.name,
            nodes,
            self.connections,
            entry_node,
            input_pins,
            output_pins,
            default_values,
        )
        .await
    }
}

/// 编译后的蓝图
#[derive(Debug, Clone)]
pub struct CompiledBlueprint {
    pub name: String,
    pub executor: BlueprintExecutor,
    /// 节点的默认值: node_name -> (pin_name -> DataValue)
    pub node_defaults: HashMap<String, HashMap<String, DataValue>>,
    pub variable_declarations: HashSet<String>,
    pub variable_defaults: HashMap<String, DataValue>,
    /// Runtime node name -> script/source reference.
    pub source_map: HashMap<String, WorkflowSourceRef>,
    // Future: 添加 validation_report
    // pub validation_report: ValidationReport,
}

impl CompiledBlueprint {
    // Future: 重新实现
    /*
    /// 获取验证报告
    pub fn report(&self) -> &ValidationReport {
        &self.validation_report
    }
    */

    /// 获取执行器
    pub fn executor(self) -> BlueprintExecutor {
        self.executor
    }

    /// 初始化节点默认值到ExecutionContext的cache
    pub async fn initialize_defaults(
        &self,
        ctx: &mut crate::workflow::execution::ExecutionContext,
    ) -> crate::error::Result<()> {
        ctx.reset_workflow_variable_scope(&self.variable_declarations, &self.variable_defaults)
            .await?;

        for (node_name, pin_defaults) in &self.node_defaults {
            for (pin_name, value) in pin_defaults {
                // 写入 "node:defaults:pin" 供未连接的引脚回退读取
                let defaults_key = format!("{}:defaults:{}", node_name, pin_name);
                ctx.set_cached(&defaults_key, value).await?;
                // 同时写入 "node:pin" 供已连接源节点的直接查找
                // （显式 execute_with_params 传入时会覆盖此值，优先级更高）
                let primary_key = format!("{}:{}", node_name, pin_name);
                ctx.set_cached(&primary_key, value).await?;
            }
        }
        Ok(())
    }

    // Future: 重新实现
    /*
    /// 打印验证报告
    pub fn print_report(&self) {
        if !self.validation_report.warnings.is_empty() {
            tracing::debug!("⚠️  Blueprint '{}' Warnings:", self.name);
            for warning in &self.validation_report.warnings {
                tracing::debug!("  - {}", warning);
            }
        }

        if self.validation_report.is_valid() {
            tracing::debug!("✅ Blueprint '{}' is valid!", self.name);
        } else {
            tracing::debug!("❌ Blueprint '{}' has errors:", self.name);
            for error in &self.validation_report.errors {
                tracing::debug!("  - {}", error);
            }
        }
    }
    */
}

#[cfg(test)]
mod tests {
    // Future: 重写测试 - validate() 和 print_report() 方法不存在
    /*
    use super::*;

    #[test]
    fn test_builder_api() {
        // 流畅的链式 API
        let result = BlueprintBuilder::new("TestBlueprint")
            .add_add("Add1")
            .add_multiply("Mul1")
            .connect_data("Add1", "Mul1", "A")
            .set_entry("Add1")
            .validate();

        assert!(result.is_ok());
        let report = result.unwrap();
        assert!(report.is_valid());
    }

    #[test]
    fn test_builder_with_cycle() {
        // 创建循环图
        let result = BlueprintBuilder::new("CycleTest")
            .add_branch("Branch1")
            .add_branch("Branch2")
            .connect_exec("Branch1", "True", "Branch2")
            .connect_exec("Branch2", "True", "Branch1") // 循环
            .validate();

        assert!(result.is_err());
    }

    #[test]
    fn test_compile() {
        let compiled = BlueprintBuilder::new("MathBlueprint")
            .add_add("Add1")
            .add_multiply("Mul1")
            .connect_data("Add1", "Mul1", "A")
            .set_entry("Add1")
            .compile();

        assert!(compiled.is_ok());
        let blueprint = compiled.unwrap();
        blueprint.print_report();
    }
    */
}
