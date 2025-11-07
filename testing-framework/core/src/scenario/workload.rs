use std::sync::Arc;

use async_trait::async_trait;

use super::{DynError, Expectation, RunContext};

#[async_trait]
pub trait Workload: Send + Sync {
    fn name(&self) -> &str;

    fn expectations(&self) -> Vec<Arc<dyn Expectation>> {
        Vec::new()
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError>;
}
