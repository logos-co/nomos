use std::{sync::Arc, time::Duration};

use super::{expectation::Expectation, workload::Workload};
use crate::topology::{GeneratedTopology, TopologyBuilder, TopologyConfig};

/// Immutable scenario definition shared between the runner, workloads, and
/// expectations.
pub struct Scenario {
    topology: GeneratedTopology,
    workloads: Vec<Arc<dyn Workload>>,
    expectations: Vec<Arc<dyn Expectation>>,
    duration: Duration,
}

impl Scenario {
    fn new(
        topology: GeneratedTopology,
        workloads: Vec<Arc<dyn Workload>>,
        expectations: Vec<Arc<dyn Expectation>>,
        duration: Duration,
    ) -> Self {
        Self {
            topology,
            workloads,
            expectations,
            duration,
        }
    }

    #[must_use]
    pub const fn topology(&self) -> &GeneratedTopology {
        &self.topology
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
    pub const fn duration(&self) -> Duration {
        self.duration
    }
}

/// Builder used by callers to describe the desired scenario.
pub struct Builder {
    topology: TopologyBuilder,
    workloads: Vec<Arc<dyn Workload>>,
    expectations: Vec<Arc<dyn Expectation>>,
    duration: Duration,
}

impl Builder {
    #[must_use]
    pub fn new(topology: TopologyBuilder) -> Self {
        Self {
            topology,
            workloads: Vec::new(),
            expectations: Vec::new(),
            duration: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn with_node_counts(validators: usize, executors: usize) -> Self {
        Self::new(TopologyBuilder::new(TopologyConfig::with_node_numbers(
            validators, executors,
        )))
    }

    #[must_use]
    pub fn with_workload<W>(mut self, workload: W) -> Self
    where
        W: Workload + 'static,
    {
        self.expectations.extend(workload.expectations());
        self.workloads.push(Arc::new(workload));
        self
    }

    #[must_use]
    pub fn with_expectation<E>(mut self, expectation: E) -> Self
    where
        E: Expectation + 'static,
    {
        self.expectations.push(Arc::new(expectation));
        self
    }

    #[must_use]
    pub const fn with_run_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    #[must_use]
    pub fn build(self) -> Scenario {
        let Self {
            topology,
            workloads,
            expectations,
            duration,
        } = self;

        let generated = topology.build();

        Scenario::new(generated, workloads, expectations, duration)
    }
}
