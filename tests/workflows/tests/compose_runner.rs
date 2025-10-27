use std::{
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

use testing_framework_core::scenario::{
    ConsensusLiveness, Expectation, PrometheusBlockProduction, Runner as _, ScenarioBuilder,
    Workload,
};
use testing_framework_runner_compose::ComposeRunner;
use tests_workflows::{
    configure_prometheus_metrics,
    workloads::{TxWorkload, TxWorkloadConfig},
};

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
#[ignore = "requires Docker daemon"]
#[expect(
    clippy::clone_on_ref_ptr,
    reason = "need to duplicate the workload arc as both trait object and concrete type"
)]
async fn compose_runner_tx_workload() {
    if !docker_available() {
        eprintln!("Skipping compose_runner_tx_workload: Docker is unavailable");
        return;
    }

    let config = TxWorkloadConfig::default();
    let workload_impl = Arc::new(TxWorkload::new(config.clone()));
    let workload: Arc<dyn Workload> = workload_impl.clone();
    let liveness: Arc<dyn Expectation> = Arc::new(ConsensusLiveness::default());
    let tx_expectation: Arc<dyn Expectation> =
        Arc::new(workload_impl.expect_inclusion(config.transactions));
    let mut builder = ScenarioBuilder::with_node_counts(1, 1)
        .with_workload(Arc::clone(&workload))
        .with_expectation(liveness)
        .with_expectation(tx_expectation);

    let metrics = configure_prometheus_metrics();

    let blocks: Arc<dyn Expectation> = Arc::new(PrometheusBlockProduction::new(1));
    builder = builder.with_metrics(metrics).with_expectation(blocks);

    let plan = builder.with_run_duration(Duration::from_secs(1)).build();

    let runner = ComposeRunner::new();
    let _handle = runner.run(&plan).await.expect("scenario executed");
}
