//! 基础控制流节点 - 类似 UE 蓝图的控制节点
//!
//! 提供常用的执行控制节点：分支、循环、选择等

use super::blueprint::{BlueprintNode, DataValue, NodeOutput, Pin, PinContainerType};
use crate::cache::CacheExt;
use crate::error::Result;
use crate::orchestration::Context;
use async_trait::async_trait;
use std::collections::HashMap;

// ==================== 分支节点 ====================

/// Branch 节点 - 根据条件选择执行路径（类似 if-else）
///
/// 引脚：
/// - exec (in) - 执行输入
/// - condition (data in, bool) - 条件判断
/// - true (exec out) - 条件为真时执行
/// - false (exec out) - 条件为假时执行
///
/// 使用场景：
/// ```
/// if is_valid {
///     execute_success_path();
/// } else {
///     execute_error_path();
/// }
/// ```
#[derive(Debug, Clone)]
pub struct BranchNode {
    name: String,
    /// 从 cache 读取条件值的 key
    condition_cache_key: Option<String>,
}

impl BranchNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            condition_cache_key: None,
        }
    }

    /// 设置从 cache 读取条件的 key
    pub fn with_condition_cache(mut self, cache_key: impl Into<String>) -> Self {
        self.condition_cache_key = Some(cache_key.into());
        self
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
            Pin::data_in("condition", "bool"),
            Pin::exec_out("true"),
            Pin::exec_out("false"),
        ]
    }

    async fn execute(
        &self,
        ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        // 优先从 cache 读取条件
        let condition = if let Some(cache_key) = &self.condition_cache_key {
            (*ctx.cache)
                .get::<bool>(cache_key)
                .await
                .unwrap_or(Some(false))
                .unwrap_or(false)
        } else {
            // 从输入数据读取
            input_data
                .get("condition")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        };

        let next_pin = if condition { "true" } else { "false" };
        Ok(NodeOutput::ExecPin(next_pin.to_string()))
    }
}

// ==================== 循环节点 ====================

/// ForEach 节点 - 遍历数组执行（泛型版本）
///
/// 使用 Wildcard Pin 实现类型安全的数组遍历：
/// - 连接时自动推导元素类型
/// - 支持任意可序列化的类型
///
/// 引脚：
/// - exec (in) - 执行输入
/// - array (wildcard in, Vec<T>) - 要遍历的数组，Wildcard 类型
/// - loop_body (exec out) - 每次迭代执行
/// - completed (exec out) - 遍历完成后执行
/// - item (wildcard out, T) - 当前遍历的元素，与 array 元素类型匹配
/// - index (data out, i64) - 当前索引
#[derive(Debug, Clone)]
pub struct ForEachNode {
    name: String,
    array_cache_key: String,
}

impl ForEachNode {
    pub fn new(name: impl Into<String>, array_cache_key: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            array_cache_key: array_cache_key.into(),
        }
    }
}

#[async_trait]
impl BlueprintNode for ForEachNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("exec"),
            // Wildcard 数组输入 - 可以接受 Vec<任意类型>
            Pin::wildcard_in("array", "T", PinContainerType::Array),
            Pin::exec_out("loop_body"),
            Pin::exec_out("completed"),
            // Wildcard 元素输出 - 类型与数组元素一致
            Pin::wildcard_out("item", "T", PinContainerType::None),
            Pin::data_out("index", "i64"),
        ]
    }

    async fn execute(
        &self,
        ctx: &Context,
        _input_pin: &str,
        _input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        // 从 cache 读取数组（带类型信息）
        let array_value_raw = (*ctx.cache).get_raw(&self.array_cache_key).await?;

        match array_value_raw {
            Some(serde_json::Value::Array(arr)) if !arr.is_empty() => {
                // 状态管理：当前索引
                let index_key = format!("{}::index", self.name);
                let current_index: usize = (*ctx.cache)
                    .get(&index_key)
                    .await
                    .unwrap_or(Some(0))
                    .unwrap_or(0);

                if current_index < arr.len() {
                    // 递增索引（为下次迭代准备）
                    (*ctx.cache)
                        .set(&index_key, &(current_index + 1), None)
                        .await?;

                    // 输出当前元素（保持 JSON 格式，类型信息由 Wildcard 处理）
                    let item_key = format!("{}::item", self.name);
                    (*ctx.cache)
                        .set_raw(&item_key, arr[current_index].clone(), None)
                        .await?;

                    // 输出当前索引
                    let index_out_key = format!("{}::index_out", self.name);
                    (*ctx.cache)
                        .set(&index_out_key, &(current_index as i64), None)
                        .await?;

                    Ok(NodeOutput::ExecPin("loop_body".to_string()))
                } else {
                    // 循环完成，重置状态
                    (*ctx.cache).delete(&index_key).await?;
                    Ok(NodeOutput::ExecPin("completed".to_string()))
                }
            }
            _ => {
                // 数组为空或不存在，直接完成
                Ok(NodeOutput::ExecPin("completed".to_string()))
            }
        }
    }
}

/// ForLoop 节点 - 按范围循环（0 到 N）
///
/// 引脚：
/// - exec (in)
/// - first_index (data in, i64) - 起始索引，默认 0
/// - last_index (data in, i64) - 结束索引
/// - loop_body (exec out)
/// - completed (exec out)
/// - index (data out, i64)
#[derive(Debug, Clone)]
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
            Pin::data_in("first_index", "i64"),
            Pin::data_in("last_index", "i64"),
            Pin::exec_out("loop_body"),
            Pin::exec_out("completed"),
            Pin::data_out("index", "i64"),
        ]
    }

    async fn execute(
        &self,
        ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let first = input_data
            .get("first_index")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let last = input_data
            .get("last_index")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        // 获取当前索引（循环状态）
        let index_key = format!("{}::current_index", self.name);
        let current: i64 = (*ctx.cache)
            .get(&index_key)
            .await
            .unwrap_or(Some(first))
            .unwrap_or(first);

        if current <= last {
            // 输出当前索引
            let output_key = format!("{}::index", self.name);
            (*ctx.cache).set(&output_key, &current, None).await?;

            // 递增索引
            (*ctx.cache).set(&index_key, &(current + 1), None).await?;

            Ok(NodeOutput::ExecPin("loop_body".to_string()))
        } else {
            // 循环完成，重置状态
            (*ctx.cache).delete(&index_key).await?;
            Ok(NodeOutput::ExecPin("completed".to_string()))
        }
    }
}

/// WhileLoop 节点 - 条件循环
///
/// 引脚：
/// - exec (in)
/// - condition (data in, bool) - 循环条件
/// - loop_body (exec out)
/// - completed (exec out)
#[derive(Debug, Clone)]
pub struct WhileLoopNode {
    name: String,
    condition_cache_key: String,
}

impl WhileLoopNode {
    pub fn new(name: impl Into<String>, condition_cache_key: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            condition_cache_key: condition_cache_key.into(),
        }
    }
}

#[async_trait]
impl BlueprintNode for WhileLoopNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("exec"),
            Pin::data_in("condition", "bool"),
            Pin::exec_out("loop_body"),
            Pin::exec_out("completed"),
        ]
    }

    async fn execute(
        &self,
        ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        // 从 cache 或输入读取条件
        let condition = (*ctx.cache)
            .get::<bool>(&self.condition_cache_key)
            .await
            .unwrap_or_else(|_| input_data.get("condition").and_then(|v| v.as_bool()))
            .unwrap_or(false);

        if condition {
            Ok(NodeOutput::ExecPin("loop_body".to_string()))
        } else {
            Ok(NodeOutput::ExecPin("completed".to_string()))
        }
    }
}

// ==================== 选择节点 ====================

/// Select 节点 - 根据索引选择值（类似三元运算符）
///
/// 引脚：
/// - index (data in, i64) - 选择索引
/// - option_0, option_1, ... (data in) - 选项值
/// - result (data out) - 选中的值
#[derive(Debug, Clone)]
pub struct SelectNode {
    name: String,
    option_count: usize,
    type_name: String,
}

impl SelectNode {
    pub fn new(name: impl Into<String>, option_count: usize, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            option_count,
            type_name: type_name.into(),
        }
    }
}

#[async_trait]
impl BlueprintNode for SelectNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        let mut pins = vec![Pin::data_in("index", "i64")];

        for i in 0..self.option_count {
            pins.push(Pin::data_in(format!("option_{}", i), &self.type_name));
        }

        pins.push(Pin::data_out("result", &self.type_name));
        pins
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let index = input_data
            .get("index")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as usize;

        let selected_key = format!("option_{}", index.min(self.option_count - 1));
        let selected_value = input_data
            .get(&selected_key)
            .cloned()
            .unwrap_or_else(|| DataValue::from_string(""));

        let mut output = HashMap::new();
        output.insert("result".to_string(), selected_value);

        Ok(NodeOutput::Data(output))
    }
}

// ==================== 序列节点 ====================

/// Sequence 节点 - 顺序执行多个分支
///
/// 引脚：
/// - exec (in)
/// - then_0, then_1, ... (exec out) - 按顺序执行
#[derive(Debug, Clone)]
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

// ==================== 延迟节点 ====================

/// Delay 节点 - 延迟执行
///
/// 引脚：
/// - exec (in)
/// - duration (data in, f64) - 延迟秒数
/// - completed (exec out)
#[derive(Debug, Clone)]
pub struct DelayNode {
    name: String,
}

impl DelayNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl BlueprintNode for DelayNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("exec"),
            Pin::data_in("duration", "f64"),
            Pin::exec_out("completed"),
        ]
    }

    async fn execute(
        &self,
        _ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let duration = input_data
            .get("duration")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);

        tokio::time::sleep(tokio::time::Duration::from_secs_f64(duration)).await;

        Ok(NodeOutput::ExecPin("completed".to_string()))
    }
}

// ==================== DoN 节点 ====================

/// DoN 节点 - 执行 N 次后停止
///
/// 引脚：
/// - exec (in)
/// - n (data in, i64) - 最大执行次数
/// - execute (exec out) - 未达到 N 次时执行
/// - completed (exec out) - 达到 N 次后执行
/// - counter (data out, i64) - 当前执行次数
#[derive(Debug, Clone)]
pub struct DoNNode {
    name: String,
}

impl DoNNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl BlueprintNode for DoNNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn pins(&self) -> Vec<Pin> {
        vec![
            Pin::exec_in("exec"),
            Pin::data_in("n", "i64"),
            Pin::exec_out("execute"),
            Pin::exec_out("completed"),
            Pin::data_out("counter", "i64"),
        ]
    }

    async fn execute(
        &self,
        ctx: &Context,
        _input_pin: &str,
        input_data: HashMap<String, DataValue>,
    ) -> Result<NodeOutput> {
        let n = input_data.get("n").and_then(|v| v.as_i64()).unwrap_or(1);

        let counter_key = format!("{}::counter", self.name);
        let counter: i64 = (*ctx.cache)
            .get(&counter_key)
            .await
            .unwrap_or(Some(0))
            .unwrap_or(0);

        let new_counter = counter + 1;
        (*ctx.cache).set(&counter_key, &new_counter, None).await?;

        // 输出当前计数
        let output_key = format!("{}::counter_out", self.name);
        (*ctx.cache).set(&output_key, &new_counter, None).await?;

        if new_counter < n {
            Ok(NodeOutput::ExecPin("execute".to_string()))
        } else {
            // 重置计数器
            (*ctx.cache).delete(&counter_key).await?;
            Ok(NodeOutput::ExecPin("completed".to_string()))
        }
    }
}
