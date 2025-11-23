use async_trait::async_trait;

use super::runner::Runner;
use crate::scenario::{DynError, Scenario};

/// Error returned when executing workloads or expectations.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("workload failure: {0}")]
    Workload(#[source] DynError),
    #[error("expectation capture failed: {0}")]
    ExpectationCapture(#[source] DynError),
    #[error("expectations failed:\n{0}")]
    Expectations(#[source] DynError),
}

#[async_trait]
pub trait Deployer<Caps = ()>: Send + Sync {
    type Error;

    async fn deploy(&self, scenario: &Scenario<Caps>) -> Result<Runner, Self::Error>;
}
