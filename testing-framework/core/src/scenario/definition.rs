use std::{sync::Arc, time::Duration};

use super::{expectation::Expectation, workload::Workload};
use crate::topology::{GeneratedTopology, TopologyBuilder, TopologyConfig};

/// Immutable scenario definition shared between the runner, workloads, and
/// expectations.
pub struct Scenario {
    topology: GeneratedTopology,
    workloads: Vec<Arc<dyn Workload>>,
    expectations: Vec<Box<dyn Expectation>>,
    duration: Duration,
}

impl Scenario {
    fn new(
        topology: GeneratedTopology,
        workloads: Vec<Arc<dyn Workload>>,
        expectations: Vec<Box<dyn Expectation>>,
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
    pub fn expectations(&self) -> &[Box<dyn Expectation>] {
        &self.expectations
    }

    #[must_use]
    pub fn expectations_mut(&mut self) -> &mut [Box<dyn Expectation>] {
        &mut self.expectations
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
    expectations: Vec<Box<dyn Expectation>>,
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
        self.expectations.push(Box::new(expectation));
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
            mut workloads,
            mut expectations,
            duration,
        } = self;

        let generated = topology.build();
        let duration = enforce_min_duration(&generated, duration);
        initialize_components(&generated, duration, &mut workloads, &mut expectations);

        Scenario::new(generated, workloads, expectations, duration)
    }
}

fn initialize_components(
    descriptors: &GeneratedTopology,
    run_duration: Duration,
    workloads: &mut [Arc<dyn Workload>],
    expectations: &mut [Box<dyn Expectation>],
) {
    initialize_workloads(descriptors, run_duration, workloads);
    initialize_expectations(descriptors, run_duration, expectations);
}

fn initialize_workloads(
    descriptors: &GeneratedTopology,
    run_duration: Duration,
    workloads: &mut [Arc<dyn Workload>],
) {
    for workload in workloads {
        let inner =
            Arc::get_mut(workload).expect("workload unexpectedly cloned before initialization");
        if let Err(err) = inner.init(descriptors, run_duration) {
            panic!("workload '{}' failed to initialize: {err}", inner.name());
        }
    }
}

fn initialize_expectations(
    descriptors: &GeneratedTopology,
    run_duration: Duration,
    expectations: &mut [Box<dyn Expectation>],
) {
    for expectation in expectations {
        if let Err(err) = expectation.init(descriptors, run_duration) {
            panic!(
                "expectation '{}' failed to initialize: {err}",
                expectation.name()
            );
        }
    }
}

fn enforce_min_duration(descriptors: &GeneratedTopology, requested: Duration) -> Duration {
    const MIN_BLOCKS: u32 = 2;
    const FALLBACK_SECS: u64 = 10;

    let slot_duration = descriptors
        .validators()
        .first()
        .map(|node| node.general.time_config.slot_duration)
        .filter(|duration| !duration.is_zero());

    let min_duration = slot_duration.map_or_else(
        || Duration::from_secs(FALLBACK_SECS),
        |slot| slot * MIN_BLOCKS,
    );

    requested.max(min_duration)
}
