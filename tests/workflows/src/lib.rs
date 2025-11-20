use testing_framework_core::scenario::Metrics;

pub mod builder;
pub mod expectations;
mod util;
pub mod workloads;

pub use builder::ScenarioBuilderExt;

/// Metrics are currently disabled in this branch; return a stub handle.
#[must_use]
pub const fn configure_prometheus_metrics() -> Metrics {
    Metrics::empty()
}
