//! Scenario orchestration primitives shared by integration tests and runners.

mod definition;
mod expectation;
pub mod http_probe;
mod runtime;
mod workload;

pub type DynError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub use definition::{Builder as ScenarioBuilder, Scenario};
pub use expectation::Expectation;
pub use runtime::{
    CleanupGuard, Deployer, NodeClients, RunContext, RunHandle, Runner, ScenarioError,
    metrics::{
        CONSENSUS_PROCESSED_BLOCKS, CONSENSUS_TRANSACTIONS_TOTAL, Metrics, MetricsError,
        PrometheusEndpoint, PrometheusInstantSample,
    },
};
pub use workload::Workload;
