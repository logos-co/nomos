use testing_framework_core::scenario::Metrics;

pub mod builder;
pub mod expectations;

pub use builder::ScenarioBuilderExt;
pub use testing_framework_workflows::{util, workloads};

/// Metrics are currently disabled in this branch; return a stub handle.
#[must_use]
pub const fn configure_prometheus_metrics() -> Metrics {
    Metrics::empty()
}
