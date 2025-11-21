use std::time::Duration;

use serial_test::serial;
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use tests_workflows::ScenarioBuilderExt as _;

const RUN_DURATION: Duration = Duration::from_secs(60);
const VALIDATORS: usize = 1;
const EXECUTORS: usize = 1;
const MIXED_TXS_PER_BLOCK: u64 = 5;

#[tokio::test]
#[serial]
/// Drives both workloads concurrently to mimic a user mixing transaction flow
/// with blob publishing on the same topology.
async fn local_runner_mixed_workloads() {
    println!(
        "running mixed workloads with {VALIDATORS} validators / {EXECUTORS} executors ({MIXED_TXS_PER_BLOCK} txs/block) for {RUN_DURATION:?}",
    );

    let topology = ScenarioBuilder::with_node_counts(VALIDATORS, EXECUTORS)
        .topology()
        .validators(VALIDATORS)
        .executors(EXECUTORS)
        .network_star()
        .apply();

    let workloads = topology
        .initialize_wallet(100, 4)
        .wallet_transactions()
        .rate(4)
        .apply()
        .transactions()
        .rate(MIXED_TXS_PER_BLOCK)
        .apply()
        .da()
        .rate(1)
        .blob_rate(1)
        .apply();

    let mut plan = workloads
        .expect_consensus_liveness()
        .with_run_duration(RUN_DURATION)
        .build();

    let deployer = LocalDeployer::default();
    let runner: Runner = deployer.deploy(&plan).await.expect("scenario deployment");
    let _handle = runner.run(&mut plan).await.expect("scenario executed");
}
