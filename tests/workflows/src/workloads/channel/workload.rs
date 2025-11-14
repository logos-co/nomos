use std::sync::Arc;

use async_trait::async_trait;
use testing_framework_core::scenario::{
    DynError, Expectation, RunContext, Workload as ScenarioWorkload,
};

use super::{
    expectation::{ChannelWorkloadExpectation, ChannelWorkloadState},
    flow::run_channel_flow,
};

#[derive(Clone, Default)]
pub struct Workload {
    state: Arc<ChannelWorkloadState>,
}

#[async_trait]
impl ScenarioWorkload for Workload {
    fn name(&self) -> &'static str {
        "channel_workload"
    }

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        vec![Box::new(ChannelWorkloadExpectation::new(Arc::clone(
            &self.state,
        )))]
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        self.state.reset();
        run_channel_flow(ctx).await?;
        self.state.mark_success();
        Ok(())
    }
}
