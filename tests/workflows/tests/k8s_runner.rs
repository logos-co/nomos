use std::time::Duration;

use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_k8s::{K8sRunner, K8sRunnerError};
use tests_workflows::workloads::transaction;

#[tokio::test]
#[ignore = "requires access to a Kubernetes cluster"]
async fn k8s_runner_tx_workload() {
    let plan = ScenarioBuilder::with_node_counts(1, 1)
        .with_workload(transaction::Workload::default())
        .with_run_duration(Duration::from_secs(15))
        .build();

    let deployer = K8sRunner::new();
    let runner: Runner = match deployer.deploy(&plan).await {
        Ok(runner) => runner,
        Err(K8sRunnerError::ClientInit { source }) => {
            eprintln!("Skipping k8s_runner_tx_workload: Kubernetes cluster unavailable ({source})");
            return;
        }
        Err(err) => panic!("scenario deployment failed: {err}"),
    };

    let context = runner.context();
    assert!(
        context.telemetry().is_configured(),
        "k8s runner should expose prometheus metrics"
    );
}
