use async_trait::async_trait;

use super::{DynError, RunContext, runtime::context::RunMetrics};
use crate::topology::GeneratedTopology;

#[async_trait]
pub trait Expectation: Send + Sync {
    fn name(&self) -> &str;

    fn init(
        &mut self,
        _descriptors: &GeneratedTopology,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        Ok(())
    }

    async fn start_capture(&mut self, _ctx: &RunContext) -> Result<(), DynError> {
        Ok(())
    }

    async fn evaluate(&mut self, ctx: &RunContext) -> Result<(), DynError>;
}
