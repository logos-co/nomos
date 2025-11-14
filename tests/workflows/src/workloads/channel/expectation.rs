use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};

pub(super) struct ChannelWorkloadExpectation {
    state: Arc<ChannelWorkloadState>,
}

#[derive(Default)]
pub(super) struct ChannelWorkloadState {
    completed: AtomicBool,
}

impl ChannelWorkloadState {
    pub fn reset(&self) {
        self.completed.store(false, Ordering::SeqCst);
    }

    pub fn mark_success(&self) {
        self.completed.store(true, Ordering::SeqCst);
    }

    fn is_completed(&self) -> bool {
        self.completed.load(Ordering::SeqCst)
    }
}

impl ChannelWorkloadExpectation {
    pub const fn new(state: Arc<ChannelWorkloadState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Expectation for ChannelWorkloadExpectation {
    fn name(&self) -> &'static str {
        "channel_workload_completed"
    }

    async fn evaluate(&mut self, _ctx: &RunContext) -> Result<(), DynError> {
        if self.state.is_completed() {
            Ok(())
        } else {
            Err("channel workload did not complete channel flow".into())
        }
    }
}
