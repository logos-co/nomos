use std::time::Duration;

use serial_test::serial;
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_compose::{ComposeRunner, ComposeRunnerError};
use tests_workflows::{ChaosBuilderExt as _, ScenarioBuilderExt as _};

const RUN_DURATION: Duration = Duration::from_secs(120);
const VALIDATORS: usize = 1;
const EXECUTORS: usize = 1;
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 64;
const TRANSACTION_WALLETS: usize = 8;

#[tokio::test]
#[serial]
async fn compose_runner_mixed_workloads() {
    let topology = ScenarioBuilder::with_node_counts(VALIDATORS, EXECUTORS)
        .enable_node_control()
        .chaos_random_restart()
        .min_delay(Duration::from_secs(45))
        .max_delay(Duration::from_secs(75))
        .apply()
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
    let runner: Runner = match deployer.deploy(&plan).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            eprintln!("Skipping compose_runner_mixed_workloads: Docker is unavailable");
            return;
        }
        Err(err) => panic!("scenario deployment: {err}"),
    };
    let context = runner.context();
    assert!(
        context.telemetry().is_configured(),
        "compose runner should expose prometheus metrics"
    );

    let _handle = runner.run(&mut plan).await.expect("scenario executed");
}
