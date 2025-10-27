use std::{env, sync::Arc, time::Duration};

use futures::FutureExt as _;
use kube::Client;
use reqwest::Url;
use testing_framework_core::{
    scenario::{
        CleanupGuard, ExecutionPlan, ExpectationError, HttpNodeClient, Metrics, NodeClientHandle,
        RunContext, RunHandle, Runner, WorkloadError,
    },
    topology::GeneratedTopology,
};
use tracing::info;
use uuid::Uuid;

use crate::{
    assets::prepare_assets,
    cleanup::RunnerCleanup,
    helm::install_release,
    logs::dump_namespace_logs,
    wait::{NodeConfigPorts, wait_for_cluster_ready},
};

pub struct K8sRunner;

impl Default for K8sRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl K8sRunner {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[derive(Default)]
struct PortSpecs {
    validators: Vec<NodeConfigPorts>,
    executors: Vec<NodeConfigPorts>,
}

struct ClusterContext {
    client: Client,
    namespace: String,
    cleanup: Option<RunnerCleanup>,
    validator_api_ports: Vec<u16>,
    validator_testing_ports: Vec<u16>,
    executor_api_ports: Vec<u16>,
    executor_testing_ports: Vec<u16>,
    prometheus_url: String,
}

/// Prometheus endpoint that must be reachable from the machine running the
/// tests. When targeting remote clusters ensure the service is port-forwarded
/// or otherwise exposed before invoking the runner.
const PROMETHEUS_URL_ENV: &str = "TEST_FRAMEWORK_PROMETHEUS_URL";

#[derive(Debug, thiserror::Error)]
pub enum K8sRunnerError {
    #[error(
        "kubernetes runner requires at least one validator and one executor (validators={validators}, executors={executors})"
    )]
    UnsupportedTopology { validators: usize, executors: usize },
    #[error("failed to initialise kubernetes client: {0}")]
    ClientInit(String),
    #[error("failed to prepare runtime assets: {0}")]
    Assets(String),
    #[error("helm operation failed: {0}")]
    Helm(String),
    #[error("kubernetes api error: {0}")]
    Kube(String),
    #[error("validator readiness probe failed: {0}")]
    Probe(String),
    #[error("workload `{name}` failed: {source}")]
    WorkloadFailed { name: String, source: WorkloadError },
    #[error("expectation `{name}` failed: {source}")]
    ExpectationFailed {
        name: String,
        source: ExpectationError,
    },
}

impl Runner for K8sRunner {
    type Error = K8sRunnerError;

    fn run<'a>(
        &'a self,
        plan: &'a ExecutionPlan,
    ) -> testing_framework_core::scenario::BoxFuture<'a, Result<RunHandle, Self::Error>> {
        async move {
            let descriptors = plan.build_topology();
            ensure_supported_topology(&descriptors)?;

            let client = Client::try_default()
                .await
                .map_err(|err| K8sRunnerError::ClientInit(err.to_string()))?;
            info!(
                "[k8s-runner] starting scenario: validators={}, executors={}",
                descriptors.validators().len(),
                descriptors.executors().len()
            );

            let port_specs = collect_port_specs(&descriptors);
            let mut cluster = setup_cluster(&client, &port_specs, &descriptors).await?;

            ensure_cluster_readiness(plan, &descriptors, &mut cluster).await?;

            let validator_clients =
                build_validator_clients(&descriptors, &cluster.validator_api_ports)?;
            let metrics = resolve_metrics(plan, Some(&cluster.prometheus_url));
            configure_plan_metrics(plan, &metrics);
            let run_duration = plan.duration();

            let context = RunContext::new(
                descriptors,
                None,
                validator_clients,
                metrics.clone(),
                run_duration,
            );

            run_workloads(plan, &context, &mut cluster).await?;
            wait_for_run_duration(run_duration).await;
            evaluate_expectations(plan, &context, &mut cluster).await?;

            let cleanup_guard: Box<dyn CleanupGuard> = Box::new(
                cluster
                    .cleanup
                    .take()
                    .expect("cleanup guard should be available"),
            );
            drop(cluster);
            info!("[k8s-runner] scenario completed; returning handle");
            Ok(RunHandle::new(context, Some(cleanup_guard)))
        }
        .boxed()
    }
}

fn ensure_supported_topology(descriptors: &GeneratedTopology) -> Result<(), K8sRunnerError> {
    let validators = descriptors.validators().len();
    let executors = descriptors.executors().len();
    if validators == 0 || executors == 0 {
        return Err(K8sRunnerError::UnsupportedTopology {
            validators,
            executors,
        });
    }
    Ok(())
}

fn collect_port_specs(descriptors: &GeneratedTopology) -> PortSpecs {
    let validators = descriptors
        .validators()
        .iter()
        .map(|node| NodeConfigPorts {
            api: node.general.api_config.address.port(),
            testing: node.general.api_config.testing_http_address.port(),
        })
        .collect();
    let executors = descriptors
        .executors()
        .iter()
        .map(|node| NodeConfigPorts {
            api: node.general.api_config.address.port(),
            testing: node.general.api_config.testing_http_address.port(),
        })
        .collect();

    PortSpecs {
        validators,
        executors,
    }
}

async fn setup_cluster(
    client: &Client,
    specs: &PortSpecs,
    descriptors: &GeneratedTopology,
) -> Result<ClusterContext, K8sRunnerError> {
    let assets =
        prepare_assets(descriptors).map_err(|err| K8sRunnerError::Assets(err.to_string()))?;
    let validators = descriptors.validators().len();
    let executors = descriptors.executors().len();

    let run_id = Uuid::new_v4().simple().to_string();
    let namespace = format!("nomos-k8s-{run_id}");
    let release = namespace.clone();

    info!(
        "[k8s-runner] installing helm release `{}` in namespace `{}`",
        release, namespace
    );
    install_release(&assets, &release, &namespace, validators, executors)
        .await
        .map_err(K8sRunnerError::Helm)?;
    info!(
        "[k8s-runner] helm install succeeded for release `{}`",
        release
    );

    let preserve = env::var("K8S_RUNNER_PRESERVE").is_ok();
    let mut cleanup_guard = Some(RunnerCleanup::new(
        client.clone(),
        namespace.clone(),
        release.clone(),
        preserve,
    ));

    let cluster_ports = match wait_for_cluster_ready(
        client,
        &namespace,
        &release,
        &specs.validators,
        &specs.executors,
    )
    .await
    {
        Ok(ports) => ports,
        Err(err) => {
            cleanup_pending(client, &namespace, &mut cleanup_guard).await;
            return Err(K8sRunnerError::Probe(err));
        }
    };

    let prometheus_url = format!("http://127.0.0.1:{}", cluster_ports.prometheus);
    info!(
        "[k8s-runner] discovered prometheus endpoint at {}",
        prometheus_url
    );

    Ok(ClusterContext {
        client: client.clone(),
        namespace,
        cleanup: cleanup_guard,
        validator_api_ports: cluster_ports
            .validators
            .iter()
            .map(|ports| ports.api)
            .collect(),
        validator_testing_ports: cluster_ports
            .validators
            .iter()
            .map(|ports| ports.testing)
            .collect(),
        executor_api_ports: cluster_ports
            .executors
            .iter()
            .map(|ports| ports.api)
            .collect(),
        executor_testing_ports: cluster_ports
            .executors
            .iter()
            .map(|ports| ports.testing)
            .collect(),
        prometheus_url,
    })
}

async fn ensure_cluster_readiness(
    plan: &ExecutionPlan,
    descriptors: &GeneratedTopology,
    cluster: &mut ClusterContext,
) -> Result<(), K8sRunnerError> {
    if !plan.readiness_enabled() {
        return Ok(());
    }

    let validator_urls = urls_from_ports(&cluster.validator_api_ports)?;
    let executor_urls = urls_from_ports(&cluster.executor_api_ports)?;
    let validator_membership_urls = urls_from_ports(&cluster.validator_testing_ports)?;
    let executor_membership_urls = urls_from_ports(&cluster.executor_testing_ports)?;

    if let Err(err) = descriptors
        .wait_remote_readiness(
            &validator_urls,
            &executor_urls,
            Some(&validator_membership_urls),
            Some(&executor_membership_urls),
        )
        .await
    {
        fail_cluster(cluster).await;
        return Err(K8sRunnerError::Probe(err.to_string()));
    }

    Ok(())
}

fn build_validator_clients(
    descriptors: &GeneratedTopology,
    validator_ports: &[u16],
) -> Result<Vec<NodeClientHandle>, K8sRunnerError> {
    descriptors
        .validators()
        .iter()
        .cloned()
        .zip(validator_ports.iter().copied())
        .map(|(descriptor, port)| {
            let client = HttpNodeClient::new(descriptor, base_url(port)?);
            Ok(NodeClientHandle::new(Arc::new(client)))
        })
        .collect()
}

fn configure_plan_metrics(plan: &ExecutionPlan, metrics: &Metrics) {
    for workload in plan.workloads() {
        workload.configure_metrics(metrics);
    }
    for expectation in plan.expectations() {
        expectation.configure_metrics(metrics);
    }
}

async fn run_workloads(
    plan: &ExecutionPlan,
    context: &RunContext,
    cluster: &mut ClusterContext,
) -> Result<(), K8sRunnerError> {
    for workload in plan.workloads() {
        if let Err(source) = workload.start(context).await {
            fail_cluster(cluster).await;
            return Err(K8sRunnerError::WorkloadFailed {
                name: workload.name().to_owned(),
                source,
            });
        }
        info!("[k8s-runner] workload `{}` completed", workload.name());
    }
    Ok(())
}

async fn evaluate_expectations(
    plan: &ExecutionPlan,
    context: &RunContext,
    cluster: &mut ClusterContext,
) -> Result<(), K8sRunnerError> {
    for expectation in plan.expectations() {
        if let Err(source) = expectation.evaluate(context).await {
            fail_cluster(cluster).await;
            return Err(K8sRunnerError::ExpectationFailed {
                name: expectation.name().to_owned(),
                source,
            });
        }
        info!(
            "[k8s-runner] expectation `{}` satisfied",
            expectation.name()
        );
    }
    Ok(())
}

async fn wait_for_run_duration(duration: Duration) {
    if !duration.is_zero() {
        tokio::time::sleep(duration).await;
    }
}

fn urls_from_ports(ports: &[u16]) -> Result<Vec<Url>, K8sRunnerError> {
    ports.iter().map(|port| base_url(*port)).collect()
}

fn base_url(port: u16) -> Result<Url, K8sRunnerError> {
    Url::parse(&format!("http://127.0.0.1:{port}/"))
        .map_err(|err| K8sRunnerError::Probe(err.to_string()))
}

async fn fail_cluster(cluster: &mut ClusterContext) {
    dump_namespace_logs(&cluster.client, &cluster.namespace).await;
    if let Some(guard) = cluster.cleanup.take() {
        Box::new(guard).cleanup();
    }
}

async fn cleanup_pending(client: &Client, namespace: &str, guard: &mut Option<RunnerCleanup>) {
    dump_namespace_logs(client, namespace).await;
    if let Some(guard) = guard.take() {
        Box::new(guard).cleanup();
    }
}

fn resolve_metrics(plan: &ExecutionPlan, fallback_url: Option<&str>) -> Metrics {
    if let Some(url) = fallback_url {
        match Metrics::from_prometheus_str(url) {
            Ok(metrics) => return metrics,
            Err(err) => tracing::warn!(
                url = %url,
                error = %err,
                "failed to initialise prometheus metrics from cluster discovery"
            ),
        }
    }

    if plan.metrics().is_configured() {
        return plan.metrics().clone();
    }

    if let Ok(raw_url) = env::var(PROMETHEUS_URL_ENV) {
        match Metrics::from_prometheus_str(&raw_url) {
            Ok(metrics) => return metrics,
            Err(err) => tracing::warn!(
                env = PROMETHEUS_URL_ENV,
                value = %raw_url,
                error = %err,
                "failed to initialise prometheus metrics handle"
            ),
        }
    }

    Metrics::empty()
}
