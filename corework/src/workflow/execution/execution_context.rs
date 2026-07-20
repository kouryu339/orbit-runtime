//! 执行上下文 - 扩展标准 Context
//!
//! 添加执行栈、节点状态管理等功能

use crate::ai_system::AIOutput;
use crate::cache::{Cache, CacheExt};
use crate::event::EventBus;
use crate::orchestration::Context;
use crate::workflow::core::DataValue;
use crate::workflow::execution::trace::{
    data_outputs_to_json, format_trace_summary, WorkflowExecutionReport, WorkflowExecutionTrace,
    WorkflowSourceRef, WorkflowToAiMode, WorkflowTraceRecorder,
};
use crate::workflow::execution::{ExecutionFlow, NodeState, StackFrame};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTraceEntry {
    pub step: usize,
    pub node_name: String,
    pub tool_name: Option<String>,
    pub to_ai: String,
    pub error_code: Option<i32>,
}

/// 扩展的执行上下文
///
/// 在标准 Context 基础上添加：
/// - 执行栈（栈帧）
/// - 执行流管理
pub struct ExecutionContext {
    /// 基础 Context（包含 cache、event_bus、registry 等）
    inner: Context,

    /// 执行栈 - 保存函数调用状态
    stack: Vec<StackFrame>,

    /// 当前执行流
    current_flow: ExecutionFlow,

    ///
    /// 注意：这里的状态是跨调用持久化的
    /// 而栈帧中的局部变量在函数返回时销毁
    node_states: HashMap<String, NodeState>,

    /// AI-facing trace collected during one workflow execution.
    workflow_trace: Vec<WorkflowTraceEntry>,

    /// Structured trace recorder. Disabled by default to keep old execution paths cheap.
    trace_recorder: Option<WorkflowTraceRecorder>,

    /// Declared workflow variable names for this execution scope.
    workflow_variables: HashSet<String>,
}

impl ExecutionContext {
    /// 从标准 Context 创建
    pub fn from_context(ctx: Context) -> Self {
        Self {
            inner: ctx,
            stack: Vec::new(),
            current_flow: ExecutionFlow::new(),
            node_states: HashMap::new(),
            workflow_trace: Vec::new(),
            trace_recorder: None,
            workflow_variables: HashSet::new(),
        }
    }

    /// 创建新的执行上下文
    pub fn new(
        cache: Arc<dyn Cache>,
        event_bus: Arc<dyn EventBus>,
        telemetry: Arc<dyn crate::monitoring::Telemetry>,
        registry: Arc<crate::system::SystemRegistry>,
    ) -> Self {
        let ctx = Context::with_registry(cache, event_bus, telemetry, registry);
        Self::from_context(ctx)
    }

    // ==================== 栈帧管理 ====================

    /// 进入新节点（创建栈帧）
    pub fn push_frame(&mut self, frame: StackFrame) {
        self.stack.push(frame);
    }

    /// 离开节点（销毁栈帧）
    pub fn pop_frame(&mut self) -> Option<StackFrame> {
        self.stack.pop()
    }

    /// 获取当前栈帧
    pub fn current_frame(&self) -> Option<&StackFrame> {
        self.stack.last()
    }

    /// 获取当前栈帧（可变）
    pub fn current_frame_mut(&mut self) -> Option<&mut StackFrame> {
        self.stack.last_mut()
    }

    /// 获取栈深度
    pub fn stack_depth(&self) -> usize {
        self.stack.len()
    }

    // ==================== 局部变量访问 ====================

    /// 读取当前栈帧的局部变量
    pub fn get_local(&self, key: &str) -> Option<&DataValue> {
        self.current_frame()?.local_vars.get(key)
    }

    /// 设置当前栈帧的局部变量
    pub fn set_local(&mut self, key: String, value: DataValue) -> bool {
        if let Some(frame) = self.current_frame_mut() {
            frame.local_vars.insert(key, value);
            true
        } else {
            false
        }
    }

    /// 删除局部变量
    pub fn remove_local(&mut self, key: &str) -> Option<DataValue> {
        self.current_frame_mut()?.local_vars.remove(key)
    }

    // ==================== Cache 访问（委托给 inner.cache，即 ScopedCache）====================

    /// 获取 cache 引用（通常是 ScopedCache）
    pub fn cache(&self) -> &Arc<dyn Cache> {
        &self.inner.cache
    }

    /// 从 cache 读取数据（泛型方法）
    pub async fn get_cached<T: crate::cache::CacheValue>(
        &self,
        key: &str,
    ) -> crate::error::Result<Option<T>> {
        (*self.inner.cache).get(key).await
    }

    /// 向 cache 写入数据
    pub async fn set_cached<T: crate::cache::CacheValue>(
        &self,
        key: &str,
        value: &T,
    ) -> crate::error::Result<()> {
        (*self.inner.cache).set(key, value, None).await
    }

    fn workflow_variable_key(name: &str) -> String {
        format!("__workflow_var__:{}", name)
    }

    pub fn is_workflow_variable_declared(&self, name: &str) -> bool {
        self.workflow_variables.contains(name)
    }

    pub async fn get_workflow_variable(
        &self,
        name: &str,
    ) -> crate::error::Result<Option<DataValue>> {
        self.get_cached(&Self::workflow_variable_key(name)).await
    }

    pub async fn set_workflow_variable(
        &self,
        name: &str,
        value: &DataValue,
    ) -> crate::error::Result<()> {
        if !self.is_workflow_variable_declared(name) {
            return Err(crate::error::FrameworkError::WorkflowError(format!(
                "SetVarNode can only write declared workflow variables or inputs, not '${}'",
                name
            )));
        }
        self.set_cached(&Self::workflow_variable_key(name), value)
            .await
    }

    pub async fn reset_workflow_variable_scope(
        &mut self,
        declarations: &HashSet<String>,
        defaults: &HashMap<String, DataValue>,
    ) -> crate::error::Result<()> {
        self.workflow_variables = declarations.clone();
        let empty = DataValue::from_string("");
        for name in declarations {
            let value = defaults.get(name).unwrap_or(&empty);
            self.set_cached(&Self::workflow_variable_key(name), value)
                .await?;
        }
        Ok(())
    }

    /// 触发事件（委托给 inner.event_bus）
    pub async fn emit_event(&self, event: crate::event::BaseEvent) {
        let _ = self.inner.event_bus.publish(event).await;
    }

    /// 获取 event_bus 引用
    pub fn event_bus(&self) -> &Arc<dyn EventBus> {
        &self.inner.event_bus
    }

    // ==================== 执行流管理 ====================

    /// 获取当前执行流
    pub fn flow(&self) -> &ExecutionFlow {
        &self.current_flow
    }

    /// 获取当前执行流（可变）
    pub fn flow_mut(&mut self) -> &mut ExecutionFlow {
        &mut self.current_flow
    }

    /// 移动到下一个节点
    pub fn move_to_node(&mut self, node_name: String, exec_pin: String) {
        self.current_flow.move_to(node_name, exec_pin);
    }

    // ==================== 节点状态管理 ====================

    /// 获取节点状态
    pub fn get_node_state(&self, node_name: &str) -> Option<&NodeState> {
        self.node_states.get(node_name)
    }

    /// 设置节点状态
    pub fn set_node_state(&mut self, node_name: String, state: NodeState) {
        self.node_states.insert(node_name, state);
    }

    /// 删除节点状态
    pub fn remove_node_state(&mut self, node_name: &str) -> Option<NodeState> {
        self.node_states.remove(node_name)
    }

    // ==================== 访问底层 Context ====================

    /// 获取内部 Context 引用
    pub fn inner(&self) -> &Context {
        &self.inner
    }

    /// 获取内部 Context 可变引用
    pub fn inner_mut(&mut self) -> &mut Context {
        &mut self.inner
    }

    /// 获取 SystemRegistry
    pub fn registry(&self) -> Arc<crate::system::SystemRegistry> {
        // registry 字段已私有化，但 Context 提供了访问方法
        // 这里我们需要添加一个getter方法到 Context
        // 临时方案：通过 system_by_type 的实现来推断，Context 内部持有 Arc<SystemRegistry>
        // 我们在 Context 上添加 pub(crate) 可见性的 getter
        self.inner.get_registry()
    }

    /// 获取 Telemetry
    pub fn telemetry(&self) -> &Arc<dyn crate::monitoring::Telemetry> {
        &self.inner.telemetry
    }

    pub fn clear_workflow_trace(&mut self) {
        self.workflow_trace.clear();
        self.trace_recorder = None;
    }

    pub fn workflow_trace(&self) -> &[WorkflowTraceEntry] {
        &self.workflow_trace
    }

    pub fn record_tool_trace(
        &mut self,
        tool_name: impl Into<String>,
        to_ai: impl Into<String>,
        error_code: Option<i32>,
    ) {
        let tool_name = tool_name.into();
        let to_ai = to_ai.into();
        self.trace_record_ai_output(Some(to_ai.clone()), error_code.map(i64::from), None);

        let node_name = self
            .flow()
            .current_node()
            .cloned()
            .unwrap_or_else(|| "<unknown>".to_string());
        self.workflow_trace.push(WorkflowTraceEntry {
            step: self.flow().current_step(),
            node_name,
            tool_name: Some(tool_name),
            to_ai,
            error_code,
        });
    }

    pub fn workflow_trace_to_ai(&self, detailed: bool) -> String {
        if self.workflow_trace.is_empty() {
            return "Workflow executed successfully.".to_string();
        }

        if !detailed
            && self
                .workflow_trace
                .iter()
                .all(|entry| entry.error_code.unwrap_or(0) == 0)
        {
            return format!(
                "Workflow executed successfully. {} tool call(s) completed.",
                self.workflow_trace.len()
            );
        }

        let mut lines = vec!["Workflow execution trace:".to_string()];
        for entry in &self.workflow_trace {
            let tool = entry.tool_name.as_deref().unwrap_or("<node>");
            let code = entry.error_code.unwrap_or(0);
            lines.push(format!(
                "- step {} node {} tool {} error_code {}: {}",
                entry.step, entry.node_name, tool, code, entry.to_ai
            ));
        }
        lines.join("\n")
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn workflow_outputs_to_json(outputs: HashMap<String, DataValue>) -> JsonValue {
        data_outputs_to_json(&outputs)
    }

    pub fn enable_trace(
        &mut self,
        workflow_name: impl Into<String>,
        source_map: HashMap<String, WorkflowSourceRef>,
    ) {
        self.workflow_trace.clear();
        self.trace_recorder = Some(WorkflowTraceRecorder::new(workflow_name, source_map));
    }

    pub fn take_trace(&mut self) -> Option<WorkflowExecutionTrace> {
        self.trace_recorder
            .take()
            .map(WorkflowTraceRecorder::finish)
    }

    pub fn trace_begin_node(&mut self, node_name: impl Into<String>, node_type: impl Into<String>) {
        if let Some(recorder) = self.trace_recorder.as_mut() {
            recorder.begin_node(node_name, node_type);
        }
    }

    pub fn trace_finish_node(&mut self, node_name: &str, output_pin: Option<String>) {
        if let Some(recorder) = self.trace_recorder.as_mut() {
            recorder.finish_node(node_name, output_pin);
        }
    }

    pub fn trace_fail_node(&mut self, node_name: &str, error: impl Into<String>) {
        if let Some(recorder) = self.trace_recorder.as_mut() {
            recorder.fail_node(node_name, error);
        }
    }

    pub fn trace_record_ai_output(
        &mut self,
        to_ai: Option<String>,
        error_code: Option<i64>,
        result_preview: Option<JsonValue>,
    ) {
        if let Some(recorder) = self.trace_recorder.as_mut() {
            recorder.record_ai_output(to_ai, error_code, result_preview);
        }
    }

    pub fn trace_record_node_values(
        &mut self,
        node_name: &str,
        input_preview: Option<JsonValue>,
        result_preview: Option<JsonValue>,
    ) {
        if let Some(recorder) = self.trace_recorder.as_mut() {
            recorder.record_node_values(node_name, input_preview, result_preview);
        }
    }

    pub fn current_node_name(&self) -> Option<&str> {
        self.flow().current_node().map(String::as_str)
    }

    // ==================== 工作流执行 ====================

    /// 获取工作流的接口信息（输入和输出结构）
    ///
    /// 不执行工作流，只分析其定义，返回需要什么输入和会产生什么输出
    ///
    /// # 参数
    /// - `blueprint_file`: 工作流JSON文件路径
    ///
    /// # 返回
    /// - WorkflowInterface: 包含输入参数、输出参数和描述信息
    ///
    /// # 示例
    /// ```ignore
    /// let interface = exec_ctx.get_workflow_interface("test.json").await?;
    /// interface.print(); // 打印接口信息
    ///
    /// for input in &interface.inputs {
    ///     println!("需要参数: {} ({})", input.name, input.data_type);
    /// }
    /// ```
    pub async fn get_workflow_interface(
        &self,
        blueprint_file: &str,
    ) -> crate::error::Result<crate::workflow::interface::WorkflowInterface> {
        use crate::workflow::interface::{WorkflowInterface, WorkflowPin};

        // 1. 读取文件
        let file_content = std::fs::read_to_string(blueprint_file).map_err(|e| {
            crate::error::FrameworkError::SystemError(format!(
                "无法读取蓝图文件 {}: {}",
                blueprint_file, e
            ))
        })?;

        // 2. 解析JSON获取基本信息
        let json: serde_json::Value = serde_json::from_str(&file_content).map_err(|e| {
            crate::error::FrameworkError::SystemError(format!("JSON解析失败: {}", e))
        })?;

        // 3. 提取工作流名称和描述
        let name = json
            .get("metadata")
            .and_then(|m| m.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("Unknown")
            .to_string();

        let description = json
            .get("metadata")
            .and_then(|m| m.get("description"))
            .and_then(|d| d.as_str())
            .map(|s| s.to_string());

        let mut interface = WorkflowInterface::empty(name);
        interface.description = description;

        // 4. 提取节点信息
        if let Some(nodes) = json.get("nodes").and_then(|v| v.as_array()) {
            let empty_vec = vec![];
            for node in nodes {
                let node_type = node.get("node_type").and_then(|v| v.as_str()).unwrap_or("");

                let pins = node
                    .get("pins")
                    .and_then(|v| v.as_array())
                    .unwrap_or(&empty_vec);

                match node_type {
                    // StartNode的输出引脚 → 工作流的输入
                    "StartNode" => {
                        for pin in pins {
                            if let Some(kind) = pin.get("kind").and_then(|v| v.as_str()) {
                                if kind == "DataOutput" {
                                    let pin_name = pin
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown")
                                        .to_string();

                                    let data_type = pin
                                        .get("data_type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Any")
                                        .to_string();

                                    let description = pin
                                        .get("description")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());

                                    let default_value =
                                        pin.get("default_value").map(|v| v.to_string());

                                    interface = interface.add_input(WorkflowPin {
                                        name: pin_name,
                                        data_type,
                                        description,
                                        default_value,
                                    });
                                }
                            }
                        }
                    }

                    // EndNode的输入引脚 → 工作流的输出
                    "EndNode" => {
                        for pin in pins {
                            if let Some(kind) = pin.get("kind").and_then(|v| v.as_str()) {
                                if kind == "DataInput" {
                                    let pin_name = pin
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown")
                                        .to_string();

                                    let data_type = pin
                                        .get("data_type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Any")
                                        .to_string();

                                    let description = pin
                                        .get("description")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());

                                    interface = interface.add_output(WorkflowPin {
                                        name: pin_name,
                                        data_type,
                                        description,
                                        default_value: None,
                                    });
                                }
                            }
                        }
                    }

                    _ => {}
                }
            }
        }

        Ok(interface)
    }

    /// 一键执行工作流（框架级别API）
    ///
    /// # 参数
    /// - `blueprint_file`: 工作流JSON文件路径
    /// - `inputs`: 输入参数（pin_name -> value），为空则使用默认值
    ///
    /// # 返回
    /// - 工作流的输出结果（来自EndNode的数据）
    ///
    /// # 示例
    /// ```ignore
    /// let mut inputs = HashMap::new();
    /// inputs.insert("url".to_string(), DataValue::from_string("https://example.com".to_string()));
    ///
    /// let results = exec_ctx.execute_workflow("test.json", inputs).await?;
    /// ```
    pub async fn execute_workflow(
        &mut self,
        blueprint_file: &str,
        inputs: HashMap<String, DataValue>,
    ) -> crate::error::Result<HashMap<String, DataValue>> {
        use crate::workflow::blueprint_loader::BlueprintLoader;

        // 1. 读取文件
        let file_content = std::fs::read_to_string(blueprint_file).map_err(|e| {
            crate::error::FrameworkError::SystemError(format!(
                "无法读取蓝图文件 {}: {}",
                blueprint_file, e
            ))
        })?;

        // 2. 加载蓝图
        let loader = BlueprintLoader::new();
        let blueprint = loader.load_from_json_str(&file_content, &self.inner)?;

        // 3. 初始化节点默认值
        blueprint.compiled.initialize_defaults(self).await?;

        // 4. 执行工作流
        let executor = blueprint.compiled.executor();
        executor.execute_with_params(self, inputs).await
    }

    pub async fn execute_workflow_report(
        &mut self,
        blueprint_file: &str,
        inputs: HashMap<String, DataValue>,
        trace_enabled: bool,
    ) -> crate::error::Result<WorkflowExecutionReport> {
        use crate::workflow::blueprint_loader::BlueprintLoader;

        let file_content = std::fs::read_to_string(blueprint_file).map_err(|e| {
            crate::error::FrameworkError::SystemError(format!(
                "无法读取蓝图文件 {}: {}",
                blueprint_file, e
            ))
        })?;

        let loader = BlueprintLoader::new();
        let blueprint = loader.load_from_json_str(&file_content, &self.inner)?;

        if trace_enabled {
            self.enable_trace(
                blueprint.metadata.name.clone(),
                blueprint.compiled.source_map.clone(),
            );
        } else {
            self.trace_recorder = None;
        }

        blueprint.compiled.initialize_defaults(self).await?;
        let executor = blueprint.compiled.executor();
        let outputs = executor.execute_with_params(self, inputs).await?;
        let trace = self.take_trace();

        Ok(WorkflowExecutionReport { outputs, trace })
    }

    pub async fn execute_workflow_ai_output(
        &mut self,
        blueprint_file: &str,
        inputs: HashMap<String, DataValue>,
    ) -> AIOutput {
        self.clear_workflow_trace();
        match self
            .execute_workflow_report(blueprint_file, inputs, true)
            .await
        {
            Ok(report) => report.into_ai_output(WorkflowToAiMode::DetailedOnError),
            Err(err) => {
                let trace = self.take_trace();
                AIOutput::error(
                    -1,
                    format_trace_summary(trace.as_ref(), Some(&err.to_string())),
                )
            }
        }
    }
}

// 实现 Clone（如果 Context 是 Clone 的）
impl Clone for ExecutionContext {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            stack: Vec::new(),                  // 栈不克隆
            current_flow: ExecutionFlow::new(), // 执行流重置
            node_states: HashMap::new(),        // 节点状态不克隆
            workflow_trace: Vec::new(),
            trace_recorder: None,
            workflow_variables: self.workflow_variables.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::InMemoryCache;
    use crate::event::InMemoryEventBus;
    use crate::monitoring::NoopTelemetry;
    use crate::system::SystemRegistry;

    fn test_context() -> ExecutionContext {
        ExecutionContext::new(
            Arc::new(InMemoryCache::new()),
            Arc::new(InMemoryEventBus::new()),
            Arc::new(NoopTelemetry),
            Arc::new(SystemRegistry::new()),
        )
    }

    #[test]
    fn workflow_trace_records_tool_to_ai() {
        let mut ctx = test_context();
        ctx.move_to_node("node_1".to_string(), "Then".to_string());

        ctx.record_tool_trace("readFile", "read ok", Some(0));

        assert_eq!(ctx.workflow_trace().len(), 1);
        assert_eq!(ctx.workflow_trace()[0].node_name, "node_1");
        assert_eq!(
            ctx.workflow_trace()[0].tool_name.as_deref(),
            Some("readFile")
        );
        assert_eq!(
            ctx.workflow_trace_to_ai(false),
            "Workflow executed successfully. 1 tool call(s) completed."
        );
    }

    #[test]
    fn workflow_outputs_to_json_unwraps_data_values() {
        let mut outputs = HashMap::new();
        outputs.insert("message".to_string(), DataValue::from_string("hello"));
        outputs.insert("count".to_string(), DataValue::from_i64(2));

        let json = ExecutionContext::workflow_outputs_to_json(outputs);

        assert_eq!(json["message"], "hello");
        assert_eq!(json["count"], 2);
    }
}
