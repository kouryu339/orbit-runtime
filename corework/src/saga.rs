//! Saga orchestration with retry and compensation support.
//!
//! A saga is a sequence of idempotent or compensatable steps. Failed execution
//! can trigger reverse-order compensation for steps that already completed.

use crate::error::Result;
use crate::orchestration::Context;
use crate::retry::RetryPolicy;
use crate::world::OrchestrationWorld;
use async_trait::async_trait;
use std::sync::Arc;

// ============================================================================
// Saga step abstractions.
// ============================================================================

/// Saga step.
///
#[async_trait]
pub trait SagaStep: Send + Sync {
    fn name(&self) -> &str;

    /// Execute this Saga step.
    async fn execute(&self, ctx: &Context) -> Result<()>;

    async fn compensate(&self, ctx: &Context) -> Result<()>;

    fn can_skip(&self, _ctx: &Context) -> bool {
        false
    }

    fn is_idempotent(&self) -> bool {
        false
    }
}

// ============================================================================
// Saga step abstractions.
// ============================================================================

///
#[async_trait]
pub trait Saga: Send + Sync {
    fn saga_id(&self) -> &str;

    /// Execute the Saga.
    async fn execute(&self, ctx: &Context) -> Result<()>;

    async fn compensate(&self, ctx: &Context) -> Result<()>;

    fn is_recoverable(&self) -> bool {
        true
    }
}

// ============================================================================
// ============================================================================

///
pub struct SimpleSaga {
    saga_id: String,
    steps: Vec<Box<dyn SagaStep>>,
    world: Arc<OrchestrationWorld>,
    retry_policy: RetryPolicy,
    auto_compensation: bool,
}

impl SimpleSaga {
    pub fn new(saga_id: impl Into<String>, world: Arc<OrchestrationWorld>) -> Self {
        Self {
            saga_id: saga_id.into(),
            steps: vec![],
            world,
            retry_policy: RetryPolicy::default(),
            auto_compensation: true,
        }
    }

    pub fn world(&self) -> &Arc<OrchestrationWorld> {
        &self.world
    }

    pub fn add_step(mut self, step: impl SagaStep + 'static) -> Self {
        self.steps.push(Box::new(step));
        self
    }

    pub fn with_retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Disable automatic compensation.
    pub fn without_auto_compensation(mut self) -> Self {
        self.auto_compensation = false;
        self
    }

    async fn execute_step_with_retry(&self, step: &dyn SagaStep, ctx: &Context) -> Result<()> {
        let mut attempt = 0;

        loop {
            match step.execute(ctx).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    if !self.retry_policy.should_retry(attempt) {
                        return Err(e);
                    }

                    if let Some(delay) = self.retry_policy.delay_for_attempt(attempt) {
                        tracing::warn!(
                            saga_id = %self.saga_id,
                            step = %step.name(),
                            attempt = attempt + 1,
                            delay_ms = delay.as_millis(),
                            "Step failed, retrying..."
                        );
                        tokio::time::sleep(delay).await;
                        attempt += 1;
                    } else {
                        return Err(e);
                    }
                }
            }
        }
    }
}

#[async_trait]
impl Saga for SimpleSaga {
    fn saga_id(&self) -> &str {
        &self.saga_id
    }

    async fn execute(&self, ctx: &Context) -> Result<()> {
        let mut executed_steps = vec![];

        tracing::info!(saga_id = %self.saga_id, "Starting Saga execution");

        for (idx, step) in self.steps.iter().enumerate() {
            if step.can_skip(ctx) {
                tracing::info!(
                    saga_id = %self.saga_id,
                    step = %step.name(),
                    "Step skipped"
                );
                continue;
            }

            tracing::info!(
                saga_id = %self.saga_id,
                step = %step.name(),
                step_index = idx,
                total_steps = self.steps.len(),
                "Executing step"
            );

            match self.execute_step_with_retry(step.as_ref(), ctx).await {
                Ok(_) => {
                    executed_steps.push(step.as_ref());
                    tracing::info!(
                        saga_id = %self.saga_id,
                        step = %step.name(),
                        "Step completed successfully"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        saga_id = %self.saga_id,
                        step = %step.name(),
                        error = %e,
                        "Step failed after retries"
                    );

                    // Automatic compensation.
                    if self.auto_compensation {
                        tracing::warn!(
                            saga_id = %self.saga_id,
                            "Starting automatic compensation"
                        );

                        for completed_step in executed_steps.iter().rev() {
                            if let Err(comp_err) = completed_step.compensate(ctx).await {
                                tracing::error!(
                                    saga_id = %self.saga_id,
                                    step = %completed_step.name(),
                                    error = %comp_err,
                                    "Compensation failed"
                                );
                            } else {
                                tracing::info!(
                                    saga_id = %self.saga_id,
                                    step = %completed_step.name(),
                                    "Compensation completed"
                                );
                            }
                        }
                    }

                    return Err(e);
                }
            }
        }

        tracing::info!(saga_id = %self.saga_id, "Saga completed successfully");
        Ok(())
    }

    async fn compensate(&self, ctx: &Context) -> Result<()> {
        tracing::info!(saga_id = %self.saga_id, "Starting manual compensation");

        for step in self.steps.iter().rev() {
            if let Err(e) = step.compensate(ctx).await {
                tracing::error!(
                    saga_id = %self.saga_id,
                    step = %step.name(),
                    error = %e,
                    "Compensation failed"
                );
                return Err(e);
            }
        }

        tracing::info!(saga_id = %self.saga_id, "Manual compensation completed");
        Ok(())
    }
}

// ============================================================================
// ============================================================================

///
pub struct SagaBuilder {
    saga_id: String,
    steps: Vec<Box<dyn SagaStep>>,
    world: Arc<OrchestrationWorld>,
    retry_policy: RetryPolicy,
    auto_compensation: bool,
}

impl SagaBuilder {
    pub fn new(saga_id: impl Into<String>, world: Arc<OrchestrationWorld>) -> Self {
        Self {
            saga_id: saga_id.into(),
            steps: vec![],
            world,
            retry_policy: RetryPolicy::default(),
            auto_compensation: true,
        }
    }

    pub fn add_step(mut self, step: impl SagaStep + 'static) -> Self {
        self.steps.push(Box::new(step));
        self
    }

    pub fn with_retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Disable automatic compensation.
    pub fn without_auto_compensation(mut self) -> Self {
        self.auto_compensation = false;
        self
    }

    pub fn build(self) -> SimpleSaga {
        SimpleSaga {
            saga_id: self.saga_id,
            steps: self.steps,
            world: self.world,
            retry_policy: self.retry_policy,
            auto_compensation: self.auto_compensation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheExt;

    #[allow(dead_code)]
    struct TestStep {
        name: String,
        should_fail: bool,
    }

    #[async_trait]
    impl SagaStep for TestStep {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, ctx: &Context) -> Result<()> {
            if self.should_fail {
                return Err(crate::error::FrameworkError::InvalidOperation(
                    "Step failed".to_string(),
                ));
            }
            ctx.cache
                .set(&format!("executed:{}", self.name), &true, None)
                .await?;
            Ok(())
        }

        async fn compensate(&self, ctx: &Context) -> Result<()> {
            ctx.cache
                .set(&format!("compensated:{}", self.name), &true, None)
                .await?;
            Ok(())
        }
    }

    /*
    #[tokio::test]
    async fn test_saga_success() {
    }

    #[tokio::test]
    async fn test_saga_failure_with_compensation() {
    }
    */
}
