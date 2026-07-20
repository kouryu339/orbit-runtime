//! 工作流接口信息 - 描述工作流的输入和输出结构

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 工作流参数（输入或输出）的信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowPin {
    /// 引脚名称
    pub name: String,
    /// 数据类型（如 "String", "bool", "i64", "f64"）
    pub data_type: String,
    /// 描述
    pub description: Option<String>,
    /// 默认值（仅用于输入）
    pub default_value: Option<String>,
}

/// 工作流的接口信息 - 描述需要哪些输入，会产生哪些输出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInterface {
    /// 工作流名称
    pub name: String,
    /// 工作流描述
    pub description: Option<String>,

    /// 输入参数（StartNode的输出引脚）
    /// 注：StartNode的"输出"在工作流外部表现为"输入"
    pub inputs: Vec<WorkflowPin>,

    /// 输出结果（EndNode的输入引脚）
    /// 注：EndNode的"输入"在工作流外部表现为"输出"
    pub outputs: Vec<WorkflowPin>,
}

impl WorkflowInterface {
    /// 创建空的接口
    pub fn empty(name: String) -> Self {
        Self {
            name,
            description: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    /// 添加输入参数
    pub fn add_input(mut self, pin: WorkflowPin) -> Self {
        self.inputs.push(pin);
        self
    }

    /// 添加输出参数
    pub fn add_output(mut self, pin: WorkflowPin) -> Self {
        self.outputs.push(pin);
        self
    }

    /// 打印工作流接口信息（用于调试）
    pub fn print(&self) {
        println!("📋 工作流: {}", self.name);
        if let Some(desc) = &self.description {
            println!("   {}", desc);
        }

        if !self.inputs.is_empty() {
            println!("\n📥 需要的输入参数:");
            for input in &self.inputs {
                println!(
                    "   - {}: {} ({})",
                    input.name,
                    input.data_type,
                    input.description.as_deref().unwrap_or("无描述")
                );
                if let Some(default) = &input.default_value {
                    println!("     默认值: {}", default);
                }
            }
        } else {
            println!("\n📥 无需输入参数");
        }

        if !self.outputs.is_empty() {
            println!("\n📤 产生的输出结果:");
            for output in &self.outputs {
                println!(
                    "   - {}: {} ({})",
                    output.name,
                    output.data_type,
                    output.description.as_deref().unwrap_or("无描述")
                );
            }
        } else {
            println!("\n📤 无输出结果");
        }
    }

    /// 获取输入参数的类型映射
    pub fn input_types(&self) -> HashMap<String, String> {
        self.inputs
            .iter()
            .map(|pin| (pin.name.clone(), pin.data_type.clone()))
            .collect()
    }

    /// 获取输出参数的类型映射
    pub fn output_types(&self) -> HashMap<String, String> {
        self.outputs
            .iter()
            .map(|pin| (pin.name.clone(), pin.data_type.clone()))
            .collect()
    }

    /// 验证提供的输入是否匹配工作流需求
    pub fn validate_inputs(&self, provided: &[String]) -> (bool, Vec<String>) {
        let required: Vec<String> = self
            .inputs
            .iter()
            .filter(|p| p.default_value.is_none())
            .map(|p| p.name.clone())
            .collect();

        let provided_set: std::collections::HashSet<_> = provided.iter().cloned().collect();
        let required_set: std::collections::HashSet<_> = required.iter().cloned().collect();

        let missing: Vec<String> = required_set.difference(&provided_set).cloned().collect();

        (missing.is_empty(), missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_interface() {
        let interface = WorkflowInterface::empty("test".to_string())
            .add_input(WorkflowPin {
                name: "url".to_string(),
                data_type: "String".to_string(),
                description: Some("网页URL".to_string()),
                default_value: Some("https://example.com".to_string()),
            })
            .add_output(WorkflowPin {
                name: "result".to_string(),
                data_type: "String".to_string(),
                description: Some("执行结果".to_string()),
                default_value: None,
            });

        assert_eq!(interface.inputs.len(), 1);
        assert_eq!(interface.outputs.len(), 1);
    }
}
