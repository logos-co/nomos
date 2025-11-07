use async_trait::async_trait;
use testing_framework_core::{
    scenario::{
        Deployer, DynError, Metrics, NodeClients, RunContext, Runner, Scenario, ScenarioError,
    },
    topology::{ReadinessError, Topology},
};
use thiserror::Error;

/// Spawns validators and executors as local processes, reusing the existing
/// integration harness.
#[derive(Clone)]
pub struct LocalDeployer {
    membership_check: bool,
}

#[derive(Debug, Error)]
pub enum LocalDeployerError {
    #[error("readiness probe failed: {source}")]
    ReadinessFailed {
        #[source]
        source: ReadinessError,
    },
    #[error("workload failed: {source}")]
    WorkloadFailed {
        #[source]
        source: DynError,
    },
    #[error("expectations failed: {source}")]
    ExpectationsFailed {
        #[source]
        source: DynError,
    },
}

impl From<ScenarioError> for LocalDeployerError {
    fn from(value: ScenarioError) -> Self {
        match value {
            ScenarioError::Workload(source) => Self::WorkloadFailed { source },
            ScenarioError::Expectations(source) => Self::ExpectationsFailed { source },
        }
    }
}

#[async_trait]
impl Deployer for LocalDeployer {
    type Error = LocalDeployerError;

    async fn deploy(&self, scenario: &Scenario) -> Result<Runner, Self::Error> {
        let topology = Self::prepare_topology(scenario, self.membership_check).await?;
        let node_clients = NodeClients::from_topology(scenario.topology(), &topology);

        let context = RunContext::new(
            scenario.topology().clone(),
            Some(topology),
            node_clients,
            scenario.duration(),
            Metrics::empty(),
        );

        Ok(Runner::new(context, None))
    }
}

impl LocalDeployer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn with_membership_check(mut self, enabled: bool) -> Self {
        self.membership_check = enabled;
        self
    }

    async fn prepare_topology(
        scenario: &Scenario,
        membership_check: bool,
    ) -> Result<Topology, LocalDeployerError> {
        let descriptors = scenario.topology();
        let topology = descriptors.clone().spawn_local().await;

        let skip_membership = !membership_check;
        if let Err(source) = wait_for_readiness(&topology, skip_membership).await {
            return Err(LocalDeployerError::ReadinessFailed { source });
        }

        Ok(topology)
    }
}

impl Default for LocalDeployer {
    fn default() -> Self {
        Self {
            membership_check: true,
        }
    }
}

async fn wait_for_readiness(
    topology: &Topology,
    skip_membership: bool,
) -> Result<(), ReadinessError> {
    topology.wait_network_ready().await?;
    if skip_membership {
        Ok(())
    } else {
        topology.wait_membership_ready().await
    }
}
