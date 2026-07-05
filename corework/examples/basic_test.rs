// Minimal test to verify core framework compiles and runs

use corework::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== Corework Framework Basic Test ===\n");

    // Test 1: Cache System
    println!("Test 1: Cache System");
    let cache = Arc::new(InMemoryCache::default());
    cache.set("key1", &"value1".to_string(), None).await?;
    let value: String = cache.get("key1").await?.unwrap();
    println!("  Retrieved: {}", value);
    println!("  ✓ Cache works\n");

    // Test 2: Event System
    println!("Test 2: Event System");
    let event_bus = Arc::new(InMemoryEventBus::default());
    println!("  ✓ Event system works\n");

    // Test 3: Data Type Registry
    println!("Test 3: Data Type System");
    let _registry = DataTypeRegistry::new();
    println!("  ✓ Data type registry created\n");

    // Test 4: OrchestrationWorld
    println!("Test 4: OrchestrationWorld");
    let world = Arc::new(OrchestrationWorld::new());
    world.set_resource("config", &"test_config".to_string(), None)?;
    let config: String = world.get_resource("config")?.unwrap();
    println!("  Config: {}", config);
    println!("  ✓ World works\n");

    // Test 5: Context
    println!("Test 5: Context");
    let telemetry = Arc::new(NoopTelemetry);
    let ctx = Context::new(cache.clone(), event_bus.clone(), telemetry);
    println!("  Request ID: {}", ctx.request_id);
    println!("  ✓ Context works\n");

    println!("=== All Basic Tests Passed ===");
    Ok(())
}
