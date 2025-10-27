use std::{sync::Arc, time::Duration};

use testing_framework_core::scenario::{
    ConsensusLiveness, Expectation, Runner as _, ScenarioBuilder, Workload,
};
use testing_framework_runner_local::LocalRunner;
use tests_workflows::workloads::{TxWorkload, TxWorkloadConfig};

#[tokio::test]
async fn local_runner_tx_workload() {
    let config = TxWorkloadConfig::default();
    let workload: Arc<dyn Workload> = Arc::new(TxWorkload::new(config.clone()));
    let liveness: Arc<dyn Expectation> = Arc::new(ConsensusLiveness::default());
    let plan = ScenarioBuilder::with_node_counts(1, 1)
        .with_workload(Arc::clone(&workload))
        .with_expectation(liveness)
        .with_run_duration(Duration::from_secs(1))
        .build();

    let runner = LocalRunner::new();
    let _handle = runner.run(&plan).await.expect("scenario executed");
}
