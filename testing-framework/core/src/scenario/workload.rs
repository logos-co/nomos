use async_trait::async_trait;

use super::{DynError, Expectation, RunContext, runtime::context::RunMetrics};
use crate::topology::GeneratedTopology;

#[async_trait]
pub trait Workload: Send + Sync {
    fn name(&self) -> &str;

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        Vec::new()
    }

    fn init(
        &mut self,
        _descriptors: &GeneratedTopology,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        Ok(())
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError>;
}
