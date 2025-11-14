use std::time::Duration;

use serial_test::serial;
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use tests_workflows::{
    expectations::ConsensusLiveness,
    workloads::{channel, transaction},
};

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

    let tx_workload = transaction::Workload::with_rate(MIXED_TXS_PER_BLOCK)
        .expect("non-zero tx rate must succeed");

    let mut plan = ScenarioBuilder::with_node_counts(VALIDATORS, EXECUTORS)
        .with_workload(tx_workload)
        .with_workload(channel::Workload::default())
        .with_expectation(ConsensusLiveness)
        .with_run_duration(RUN_DURATION)
        .build();

    let deployer = LocalDeployer::default();
    let runner: Runner = deployer.deploy(&plan).await.expect("scenario deployment");
    let _handle = runner.run(&mut plan).await.expect("scenario executed");
}
