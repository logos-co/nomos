use std::{
    process::{Command, Stdio},
    time::Duration,
};

use serial_test::serial;
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_compose::ComposeRunner;
use tests_workflows::ScenarioBuilderExt as _;

const RUN_DURATION: Duration = Duration::from_secs(60);
const VALIDATORS: usize = 1;
const EXECUTORS: usize = 1;
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 64;
const TRANSACTION_WALLETS: usize = 8;

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
#[serial]
async fn compose_runner_mixed_workloads() {
    if !docker_available() {
        eprintln!("Skipping compose_runner_mixed_workloads: Docker is unavailable");
        return;
    }

    let topology = ScenarioBuilder::with_node_counts(VALIDATORS, EXECUTORS)
        .topology()
        .validators(VALIDATORS)
        .executors(EXECUTORS)
        .network_star()
        .apply();

    let workloads = topology
        .wallets(TOTAL_WALLETS)
        .transactions()
        .rate(MIXED_TXS_PER_BLOCK)
        .users(TRANSACTION_WALLETS)
        .apply()
        .da()
        .rate(1)
        .blob_rate(1)
        .apply();

    let mut plan = workloads
        .expect_consensus_liveness()
        .with_run_duration(RUN_DURATION)
        .build();

    let deployer = ComposeRunner::new().with_readiness(false);
    let runner: Runner = deployer.deploy(&plan).await.expect("scenario deployment");
    let context = runner.context();
    assert!(
        context.telemetry().is_configured(),
        "compose runner should expose prometheus metrics"
    );

    let _handle = runner.run(&mut plan).await.expect("scenario executed");
}
