use std::{sync::Arc, time::Duration};

use testing_framework_core::scenario::{
    ConsensusLiveness, Expectation, PrometheusBlockProduction, Runner as _, ScenarioBuilder,
    Workload,
};
use testing_framework_runner_k8s::{K8sRunner, K8sRunnerError};
use tests_workflows::{
    configure_prometheus_metrics,
    workloads::{TxWorkload, TxWorkloadConfig},
};

#[tokio::test]
#[ignore = "requires access to a Kubernetes cluster"]
#[expect(
    clippy::clone_on_ref_ptr,
    reason = "need to duplicate the workload arc as both trait object and concrete type"
)]
async fn k8s_runner_tx_workload() {
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

    let runner = K8sRunner::new();
    let _handle = match runner.run(&plan).await {
        Ok(handle) => handle,
        Err(K8sRunnerError::ClientInit(err)) => {
            eprintln!("Skipping k8s_runner_tx_workload: Kubernetes cluster unavailable ({err})");
            return;
        }
        Err(err) => panic!("scenario failed: {err}"),
    };
}
