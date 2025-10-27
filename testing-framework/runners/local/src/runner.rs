use std::{sync::Arc, time::Duration};

use futures::FutureExt as _;
use testing_framework_core::{
    scenario::{
        BoxFuture, ExecutionPlan, ExpectationError, HttpNodeClient, Metrics, NodeClientHandle,
        RunContext, RunHandle, Runner, WorkloadError, wait_for_readiness,
    },
    topology::{GeneratedTopology, ReadinessError, Topology},
};
use thiserror::Error;

/// Runner that spawns validators and executors as local processes, reusing the
/// existing integration harness.
#[derive(Default, Clone)]
pub struct LocalRunner;

#[derive(Debug, Error)]
pub enum LocalRunnerError {
    #[error("readiness probe failed: {source}")]
    ReadinessFailed {
        #[source]
        source: ReadinessError,
    },
    #[error("workload `{name}` failed: {source}")]
    WorkloadFailed {
        name: String,
        #[source]
        source: WorkloadError,
    },
    #[error("expectation `{name}` failed: {source}")]
    ExpectationFailed {
        name: String,
        #[source]
        source: ExpectationError,
    },
}

impl Runner for LocalRunner {
    type Error = LocalRunnerError;

    fn run<'a>(&'a self, plan: &'a ExecutionPlan) -> BoxFuture<'a, Result<RunHandle, Self::Error>> {
        async move {
            let (descriptors, topology) = Self::prepare_topology(plan).await?;
            let metrics = plan.metrics().clone();
            Self::configure_plan_metrics(plan, &metrics);

            let validator_clients = Self::build_validator_clients(&topology, &descriptors);
            let context = Arc::new(RunContext::new(
                descriptors,
                Some(topology),
                validator_clients,
                metrics,
                plan.duration(),
            ));

            Self::execute_workloads(plan, context.as_ref()).await?;
            Self::wait_for_duration(plan.duration()).await;
            Self::evaluate_expectations(plan, context.as_ref()).await?;

            Ok(RunHandle::from_arc(context))
        }
        .boxed()
    }
}

impl LocalRunner {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn prepare_topology(
        plan: &ExecutionPlan,
    ) -> Result<(GeneratedTopology, Topology), LocalRunnerError> {
        let descriptors = plan.build_topology();
        let topology = descriptors.clone().spawn_local().await;

        if plan.readiness_enabled()
            && let Err(source) = wait_for_readiness(&topology).await
        {
            dump_local_logs(&topology);
            return Err(LocalRunnerError::ReadinessFailed { source });
        }

        Ok((descriptors, topology))
    }

    fn build_validator_clients(
        topology: &Topology,
        descriptors: &GeneratedTopology,
    ) -> Vec<NodeClientHandle> {
        descriptors
            .validators()
            .iter()
            .cloned()
            .zip(topology.validators().iter())
            .map(|(config, validator)| {
                let client = HttpNodeClient::new(config, validator.url());
                NodeClientHandle::new(Arc::new(client))
            })
            .collect()
    }

    fn configure_plan_metrics(plan: &ExecutionPlan, metrics: &Metrics) {
        for workload in plan.workloads() {
            workload.configure_metrics(metrics);
        }

        for expectation in plan.expectations() {
            expectation.configure_metrics(metrics);
        }
    }

    async fn execute_workloads(
        plan: &ExecutionPlan,
        context: &RunContext,
    ) -> Result<(), LocalRunnerError> {
        for workload in plan.workloads() {
            if let Err(source) = workload.start(context).await {
                return Err(LocalRunnerError::WorkloadFailed {
                    name: workload.name().to_owned(),
                    source,
                });
            }
        }
        Ok(())
    }

    async fn evaluate_expectations(
        plan: &ExecutionPlan,
        context: &RunContext,
    ) -> Result<(), LocalRunnerError> {
        for expectation in plan.expectations() {
            if let Err(source) = expectation.evaluate(context).await {
                return Err(LocalRunnerError::ExpectationFailed {
                    name: expectation.name().to_owned(),
                    source,
                });
            }
        }
        Ok(())
    }

    async fn wait_for_duration(duration: Duration) {
        if !duration.is_zero() {
            tokio::time::sleep(duration).await;
        }
    }
}

fn dump_local_logs(topology: &Topology) {
    for (idx, validator) in topology.validators().iter().enumerate() {
        let logs = validator.get_logs_from_file();
        if !logs.is_empty() {
            eprintln!("[local-runner] validator-{idx} logs:\n{logs}");
        }
    }

    for (idx, executor) in topology.executors().iter().enumerate() {
        let logs = executor.get_logs_from_file();
        if !logs.is_empty() {
            eprintln!("[local-runner] executor-{idx} logs:\n{logs}");
        }
    }
}
