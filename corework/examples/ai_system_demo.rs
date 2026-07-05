use corework::prelude::*;
use serde::{Deserialize, Serialize};

#[buns_system("EchoSystem", description = "Echo a short message")]
pub struct EchoSystem;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EchoInput {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EchoOutput {
    pub echoed: String,
}

#[async_trait::async_trait]
impl SystemOperation for EchoSystem {
    type Input = EchoInput;
    type Output = EchoOutput;
    type Error = FrameworkError;

    async fn execute(&self, input: Self::Input, _ctx: &Context) -> Result<Self::Output> {
        Ok(EchoOutput {
            echoed: input.message,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let registry = SystemRegistry::new();
    registry.auto_register_all();

    println!("Registered systems:");
    for system in registry.registered_systems() {
        println!("- {}", system);
    }
    Ok(())
}
