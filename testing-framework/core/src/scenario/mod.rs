//! Scenario orchestration primitives shared by integration tests and runners.

pub mod cfgsync;
mod context;
pub mod expectations;
mod http_node_client;
pub mod metrics;
mod plan;

pub use context::{CleanupGuard, ClientError, NodeClient, NodeClientHandle, RunContext, RunHandle};
pub use expectations::{ConsensusLiveness, PrometheusBlockProduction};
pub use http_node_client::HttpNodeClient;
pub use metrics::{
    CONSENSUS_PROCESSED_BLOCKS, CONSENSUS_TRANSACTIONS_TOTAL, Metrics, MetricsError,
    PrometheusEndpoint, PrometheusInstantSample,
};
pub use plan::{
    BoxFuture, ExecutionPlan, Expectation, ExpectationError, Runner, ScenarioBuilder, Workload,
    WorkloadError, wait_for_readiness,
};
