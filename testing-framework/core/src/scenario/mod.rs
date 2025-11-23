//! Scenario orchestration primitives shared by integration tests and runners.

mod capabilities;
pub mod cfgsync;
mod definition;
mod expectation;
pub mod http_probe;
mod runtime;
mod workload;

pub type DynError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub use capabilities::{NodeControlCapability, NodeControlHandle, RequiresNodeControl};
pub use definition::{Builder, Scenario, ScenarioBuilder};
pub use expectation::Expectation;
pub use runtime::{
    BlockFeed, BlockFeedTask, BlockRecord, BlockStats, CleanupGuard, Deployer, NodeClients,
    RunContext, RunHandle, RunMetrics, Runner, ScenarioError,
    metrics::{
        CONSENSUS_PROCESSED_BLOCKS, CONSENSUS_TRANSACTIONS_TOTAL, Metrics, MetricsError,
        PrometheusEndpoint, PrometheusInstantSample,
    },
    spawn_block_feed,
};
pub use workload::Workload;
