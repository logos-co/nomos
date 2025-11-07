use std::time::Duration;

use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use tests_workflows::{expectations::ConsensusLiveness, workloads::transaction};

#[tokio::test]
/// Verifies that the local runner can deploy a 1 validator / 1 executor
/// scenario, execute the transaction workload, and satisfy the consensus
/// liveness expectation within the configured duration.
async fn local_runner_tx_workload() {
    let plan = ScenarioBuilder::with_node_counts(1, 1)
        .with_workload(transaction::Workload::default())
        .with_expectation(ConsensusLiveness)
        .with_run_duration(Duration::from_secs(15))
        .build();

    let deployer = LocalDeployer::default().with_membership_check(false);
    let runner: Runner = deployer.deploy(&plan).await.expect("scenario deployment");

    let _handle = runner.run(&plan).await.expect("scenario executed");
}
