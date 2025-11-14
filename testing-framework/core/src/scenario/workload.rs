use std::time::Duration;

use async_trait::async_trait;

use super::{DynError, Expectation, RunContext};
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
        _run_duration: Duration,
    ) -> Result<(), DynError> {
        Ok(())
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError>;
}
