use std::{sync::Arc, time::Duration};

use futures::future::BoxFuture as FuturesBoxFuture;

use super::{
    context::{RunContext, RunHandle},
    metrics::Metrics,
};
use crate::topology::{
    GeneratedTopology, ReadinessError, Topology, TopologyBuilder, TopologyConfig,
};

/// Convenient boxed future alias used by trait methods.
pub type BoxFuture<'a, T> = FuturesBoxFuture<'a, T>;

/// Runner is responsible for materializing the generated topology in a specific
/// environment (local processes, docker containers, k8s, etc.).
pub trait Runner: Send + Sync {
    type Error;

    fn run<'a>(&'a self, plan: &'a ExecutionPlan) -> BoxFuture<'a, Result<RunHandle, Self::Error>>;
}

/// Workloads encapsulate background activities that drive the system under test
/// (transaction generators, DA blob publishers, chaos modules, etc.).
pub trait Workload: Send + Sync {
    fn name(&self) -> &str;

    fn configure_metrics(&self, _metrics: &Metrics) {}

    fn start<'a>(&'a self, ctx: &'a RunContext) -> BoxFuture<'a, Result<(), WorkloadError>>;
}

/// Expectations evaluate cluster behaviour after or during execution.
pub trait Expectation: Send + Sync {
    fn name(&self) -> &str;

    fn configure_metrics(&self, _metrics: &Metrics) {}

    fn evaluate<'a>(&'a self, ctx: &'a RunContext) -> BoxFuture<'a, Result<(), ExpectationError>>;
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct WorkloadError {
    message: String,
}

impl WorkloadError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ExpectationError {
    message: String,
}

impl ExpectationError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Builder used by callers to describe the desired scenario.
pub struct ScenarioBuilder {
    topology: TopologyBuilder,
    workloads: Vec<Arc<dyn Workload>>,
    expectations: Vec<Arc<dyn Expectation>>,
    metrics: Option<Metrics>,
    duration: Duration,
    readiness: bool,
}

impl ScenarioBuilder {
    #[must_use]
    pub fn new(topology: TopologyBuilder) -> Self {
        Self {
            topology,
            workloads: Vec::new(),
            expectations: Vec::new(),
            metrics: None,
            duration: Duration::ZERO,
            readiness: true,
        }
    }

    #[must_use]
    pub fn with_node_counts(validators: usize, executors: usize) -> Self {
        Self::new(TopologyBuilder::new(TopologyConfig::with_node_numbers(
            validators, executors,
        )))
    }

    #[must_use]
    pub fn with_workload(mut self, workload: Arc<dyn Workload>) -> Self {
        self.workloads.push(workload);
        self
    }

    #[must_use]
    pub fn with_expectation(mut self, expectation: Arc<dyn Expectation>) -> Self {
        self.expectations.push(expectation);
        self
    }

    #[must_use]
    pub fn with_metrics(mut self, metrics: Metrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    #[must_use]
    pub const fn with_run_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    #[must_use]
    pub const fn with_readiness(mut self, enabled: bool) -> Self {
        self.readiness = enabled;
        self
    }

    #[must_use]
    pub fn build(self) -> ExecutionPlan {
        let Self {
            topology,
            workloads,
            expectations,
            metrics,
            duration,
            readiness,
        } = self;

        ExecutionPlan {
            topology,
            workloads,
            expectations,
            metrics: metrics.unwrap_or_else(Metrics::empty),
            duration,
            readiness,
        }
    }
}

/// Immutable execution plan shared between the runner, workloads, and
/// expectations.
pub struct ExecutionPlan {
    topology: TopologyBuilder,
    workloads: Vec<Arc<dyn Workload>>,
    expectations: Vec<Arc<dyn Expectation>>,
    metrics: Metrics,
    duration: Duration,
    readiness: bool,
}

impl ExecutionPlan {
    #[must_use]
    pub const fn topology(&self) -> &TopologyBuilder {
        &self.topology
    }

    #[must_use]
    pub fn build_topology(&self) -> GeneratedTopology {
        self.topology.clone().build()
    }

    #[must_use]
    pub fn workloads(&self) -> &[Arc<dyn Workload>] {
        &self.workloads
    }

    #[must_use]
    pub fn expectations(&self) -> &[Arc<dyn Expectation>] {
        &self.expectations
    }

    #[must_use]
    pub const fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    #[must_use]
    pub const fn readiness_enabled(&self) -> bool {
        self.readiness
    }
}

pub async fn wait_for_readiness(topology: &Topology) -> Result<(), ReadinessError> {
    topology.wait_network_ready().await?;
    topology.wait_membership_ready().await
}
