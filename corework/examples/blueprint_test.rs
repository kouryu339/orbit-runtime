use corework::prelude::*;
use corework::workflow::blueprint::{BlueprintExecutor, BlueprintWorkflow, EntryNode, TaskNode};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let cache = Arc::new(InMemoryCache::default());
    let event_bus = Arc::new(InMemoryEventBus::default());
    let telemetry = Arc::new(NoopTelemetry);
    let ctx = Context::new(cache, event_bus, telemetry);

    let workflow = BlueprintWorkflow::builder("example")
        .entry("start")
        .add_node(Arc::new(EntryNode::new("start")))
        .add_node(Arc::new(TaskNode::new("task", "demo", "run")))
        .connect("start", "exec", "task", "exec")
        .build()?;

    BlueprintExecutor::new(workflow).execute(&ctx).await?;
    println!("blueprint example completed");
    Ok(())
}
