use std::{
    process::{Command, Stdio},
    time::Duration,
};

use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_compose::ComposeRunner;
use tests_workflows::workloads::transaction;

fn docker_available() -> bool {
    Command::new("docker")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn compose_runner_tx_workload() {
    if !docker_available() {
        eprintln!("Skipping compose_runner_tx_workload: Docker is unavailable");
        return;
    }

    let plan = ScenarioBuilder::with_node_counts(1, 1)
        .with_workload(transaction::Workload::default())
        .with_run_duration(Duration::from_secs(15))
        .build();

    let deployer = ComposeRunner::new().with_readiness(false);
    let runner: Runner = deployer.deploy(&plan).await.expect("scenario deployment");
    let context = runner.context();
    assert!(
        context.telemetry().is_configured(),
        "compose runner should expose prometheus metrics"
    );
}
