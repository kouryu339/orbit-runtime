//! 元数据字段简单测试

use corework::prelude::*;
use serde::{Deserialize, Serialize};

// 测试1：最简单的只有元数据字段，没有params
#[buns_system("SimpleTest", description = "简单测试", readonly = true)]
pub struct SimpleTestSystem;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleInput {
    pub value: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleOutput {
    pub result: i32,
}

#[async_trait::async_trait]
impl SystemOperation for SimpleTestSystem {
    type Input = SimpleInput;
    type Output = SimpleOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: Self::Input, _ctx: &Context) -> Result<Self::Output> {
        Ok(SimpleOutput {
            result: input.value,
        })
    }
}

fn main() {
    println!("元数据测试成功！");
    let systems = SystemRegistry::list_ai_systems();
    for system in systems {
        println!("System: {}", system.name);
        println!("  ReadOnly: {}", system.readonly);
        println!("  Destructive: {}", system.destructive);
    }
}
