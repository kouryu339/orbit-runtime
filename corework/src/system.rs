//! System operation registry and inventory-backed registration.

use crate::error::FrameworkError;
use async_trait::async_trait;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

pub struct SystemFactory {
    pub name: &'static str,
    pub description: Option<&'static str>,
    pub constructor: fn() -> Arc<dyn Any + Send + Sync>,
    pub dynamic_constructor: Option<fn() -> Arc<dyn crate::workflow::dynamic_node::DynamicExecute>>,
}

inventory::collect!(SystemFactory);

///
#[async_trait]
pub trait SystemOperation: Send + Sync {
    type Input: Send + Sync;
    type Output: Send + Sync;
    type Error: Into<FrameworkError> + Send;

    ///
    async fn execute(
        &self,
        input: Self::Input,
        ctx: &crate::orchestration::Context,
    ) -> std::result::Result<Self::Output, Self::Error>;

    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }

    /// Whether this operation is idempotent.
    fn is_idempotent(&self) -> bool {
        false
    }

    fn timeout_ms(&self) -> Option<u64> {
        Some(30000)
    }
}

pub trait AutoRegisterSystem: SystemOperation + Sized + 'static {
    fn type_name() -> &'static str {
        std::any::type_name::<Self>()
    }

    fn short_name() -> String {
        let full_name = Self::type_name();
        full_name
            .split("::")
            .last()
            .unwrap_or(full_name)
            .to_string()
    }
}

///
pub struct SystemRegistry {
    operations: Arc<parking_lot::RwLock<HashMap<String, Arc<dyn std::any::Any + Send + Sync>>>>,
    dynamic_executors: Arc<
        parking_lot::RwLock<
            HashMap<String, Arc<dyn crate::workflow::dynamic_node::DynamicExecute>>,
        >,
    >,
}

impl SystemRegistry {
    pub fn new() -> Self {
        Self {
            operations: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            dynamic_executors: Arc::new(parking_lot::RwLock::new(HashMap::new())),
        }
    }

    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// let mut registry = SystemRegistry::new();
    /// registry.register("save_order", SaveOrderOperation);
    /// ```
    pub fn register<T>(&self, name: impl Into<String>, operation: T)
    where
        T: SystemOperation + crate::workflow::dynamic_node::DynamicExecute + 'static,
    {
        let name = name.into();
        let instance = Arc::new(operation);
        self.operations
            .write()
            .insert(name.clone(), instance.clone());
        self.dynamic_executors.write().insert(name, instance);
    }

    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// registry.register_type(SaveOrderOperation);
    /// ```
    pub fn register_type<T>(&self, operation: T)
    where
        T: SystemOperation + crate::workflow::dynamic_node::DynamicExecute + 'static,
    {
        let type_name = std::any::type_name::<T>()
            .split("::")
            .last()
            .unwrap_or(std::any::type_name::<T>())
            .to_string();

        let instance = Arc::new(operation);
        self.operations
            .write()
            .insert(type_name.clone(), instance.clone());
        self.dynamic_executors.write().insert(type_name, instance);
    }

    pub fn auto_register_all(&self) {
        for factory in inventory::iter::<SystemFactory>() {
            let instance = (factory.constructor)();
            self.operations
                .write()
                .insert(factory.name.to_string(), instance);

            if let Some(dynamic_constructor) = factory.dynamic_constructor {
                let dyn_instance = dynamic_constructor();
                self.dynamic_executors
                    .write()
                    .insert(factory.name.to_string(), dyn_instance);
            }
        }
    }

    pub fn list_ai_systems() -> Vec<&'static crate::ai_system::AISystemMetadata> {
        inventory::iter::<crate::ai_system::AISystemFactory>()
            .map(|f| &f.metadata)
            .collect()
    }

    pub fn get_ai_system_help(name: &str) -> Option<String> {
        inventory::iter::<crate::ai_system::AISystemFactory>()
            .find(|f| f.metadata.name == name)
            .map(|f| Self::format_help(&f.metadata))
    }

    pub fn generate_ai_help_doc() -> String {
        let mut doc = String::from("# Available AI Systems\n\n");

        for factory in inventory::iter::<crate::ai_system::AISystemFactory>() {
            doc.push_str(&format!(
                "## {}\n{}\n\n**Parameters:**\n",
                factory.metadata.name, factory.metadata.description
            ));

            for param in factory.metadata.parameters {
                let required_text = if param.required {
                    "required"
                } else {
                    "optional"
                };
                let default_text = if let Some(ref default) = param.default_value {
                    format!(", default: {}", default)
                } else {
                    String::new()
                };

                doc.push_str(&format!(
                    "- `--{}` ({}, type: {}{})\n  {}\n",
                    param.name, required_text, param.param_type, default_text, param.description
                ));
            }

            doc.push('\n');
        }

        doc
    }

    fn format_help(metadata: &crate::ai_system::AISystemMetadata) -> String {
        let mut help = format!(
            "# {}\n\n{}\n\n## Parameters\n",
            metadata.name, metadata.description
        );

        for param in metadata.parameters {
            let required_text = if param.required {
                "required"
            } else {
                "optional"
            };
            let default_text = if let Some(ref default) = param.default_value {
                format!(", default: {}", default)
            } else {
                String::new()
            };

            help.push_str(&format!(
                "\n### --{}\n**Type**: {}\n**{}**{}\n\n{}\n",
                param.name, param.param_type, required_text, default_text, param.description
            ));
        }

        help
    }

    pub fn registered_systems(&self) -> Vec<String> {
        self.operations.read().keys().cloned().collect()
    }

    pub fn register_auto<T>(&self, operation: T)
    where
        T: AutoRegisterSystem + crate::workflow::dynamic_node::DynamicExecute,
    {
        let short_name = T::short_name();
        let instance = Arc::new(operation);
        self.operations
            .write()
            .insert(short_name.clone(), instance.clone());
        self.dynamic_executors.write().insert(short_name, instance);
    }
    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// let op: Arc<SaveOrderOperation> = registry.get("save_order")?;
    /// ```
    pub fn get<T>(&self, name: &str) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.operations
            .read()
            .get(name)
            .and_then(|op| Arc::clone(op).downcast::<T>().ok())
    }
    ///
    pub fn get_any(&self, name: &str) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        self.operations.read().get(name).map(Arc::clone)
    }

    pub fn get_dynamic(
        &self,
        name: &str,
    ) -> Option<Arc<dyn crate::workflow::dynamic_node::DynamicExecute>> {
        self.dynamic_executors.read().get(name).map(Arc::clone)
    }

    ///
    pub fn register_dynamic(
        &self,
        name: impl Into<String>,
        executor: Arc<dyn crate::workflow::dynamic_node::DynamicExecute>,
    ) {
        self.dynamic_executors.write().insert(name.into(), executor);
    }
    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// let op: Arc<SaveOrderOperation> = registry.get_by_type()?;
    /// ```
    pub fn get_by_type<T>(&self) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        let type_name = std::any::type_name::<T>()
            .split("::")
            .last()
            .unwrap_or(std::any::type_name::<T>());

        self.get(type_name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.operations.read().contains_key(name)
    }

    pub fn list(&self) -> Vec<String> {
        self.operations.read().keys().cloned().collect()
    }

    /// Return the number of registered operations.
    pub fn len(&self) -> usize {
        self.operations.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.operations.read().is_empty()
    }
}

impl Default for SystemRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    /*
    use super::*;
    use crate::error::FrameworkError;
    use crate::cache::InMemoryCache;

    struct TestOperation;

    #[async_trait]
    impl SystemOperation for TestOperation {
        type Input = String;
        type Output = String;
        type Error = FrameworkError;

        async fn execute(
            &self,
            input: Self::Input,
            _ctx: &crate::orchestration::Context,
        ) -> std::result::Result<Self::Output, Self::Error> {
            Ok(format!("processed: {}", input))
        }

        fn name(&self) -> &str {
            "TestOperation"
        }
    }

    #[tokio::test]
    async fn test_system_registry() {
        let mut registry = SystemRegistry::new();
        registry.register("test", TestOperation);

        let op: Option<Arc<TestOperation>> = registry.get("test");
        assert!(op.is_some());

        let cache = Arc::new(InMemoryCache::new());
        let event_bus = Arc::new(crate::event::InMemoryEventBus::new());
        let telemetry = Arc::new(crate::monitoring::NoopTelemetry);
        let ctx = crate::orchestration::Context::new(cache, event_bus, telemetry);

        let result = op.unwrap().execute("hello".to_string(), &ctx).await.unwrap();
        assert_eq!(result, "processed: hello");
    }

    #[tokio::test]
    async fn test_cached_operation() {
        let cache = Arc::new(InMemoryCache::new());
        let cached_op = CachedSystemOperation::new(
            TestOperation,
            cache.clone(),
            "test".to_string(),
            None,
        );

        let event_bus = Arc::new(crate::event::InMemoryEventBus::new());
        let telemetry = Arc::new(crate::monitoring::NoopTelemetry);
        let ctx = crate::orchestration::Context::new(cache, event_bus, telemetry);

        let result1 = cached_op.execute("input1".to_string(), &ctx).await.unwrap();
        assert_eq!(result1, "processed: input1");

        let result2 = cached_op.execute("input1".to_string(), &ctx).await.unwrap();
        assert_eq!(result2, "processed: input1");
    }
    */
}
