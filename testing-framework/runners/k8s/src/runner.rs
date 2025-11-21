use std::env;

use anyhow::Error;
use async_trait::async_trait;
use kube::Client;
use reqwest::Url;
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{
        BlockFeed, BlockFeedTask, CleanupGuard, Deployer, Metrics, MetricsError, NodeClients,
        RunContext, Runner, Scenario, http_probe::NodeRole, spawn_block_feed,
    },
    topology::{GeneratedTopology, ReadinessError},
};
use tracing::{error, info};
use url::ParseError;
use uuid::Uuid;

use crate::{
    assets::{AssetsError, RunnerAssets, prepare_assets},
    cleanup::RunnerCleanup,
    helm::{HelmError, install_release},
    host::node_host,
    logs::dump_namespace_logs,
    wait::{ClusterPorts, ClusterWaitError, NodeConfigPorts, wait_for_cluster_ready},
};

pub struct K8sRunner {
    readiness_checks: bool,
}

impl Default for K8sRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl K8sRunner {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            readiness_checks: true,
        }
    }

    #[must_use]
    pub const fn with_readiness(mut self, enabled: bool) -> Self {
        self.readiness_checks = enabled;
        self
    }
}

#[derive(Default)]
struct PortSpecs {
    validators: Vec<NodeConfigPorts>,
    executors: Vec<NodeConfigPorts>,
}

struct ClusterEnvironment {
    client: Client,
    namespace: String,
    release: String,
    cleanup: Option<RunnerCleanup>,
    validator_api_ports: Vec<u16>,
    validator_testing_ports: Vec<u16>,
    executor_api_ports: Vec<u16>,
    executor_testing_ports: Vec<u16>,
    prometheus_port: u16,
}

impl ClusterEnvironment {
    fn new(
        client: Client,
        namespace: String,
        release: String,
        cleanup: RunnerCleanup,
        ports: &ClusterPorts,
    ) -> Self {
        Self {
            client,
            namespace,
            release,
            cleanup: Some(cleanup),
            validator_api_ports: ports.validators.iter().map(|ports| ports.api).collect(),
            validator_testing_ports: ports.validators.iter().map(|ports| ports.testing).collect(),
            executor_api_ports: ports.executors.iter().map(|ports| ports.api).collect(),
            executor_testing_ports: ports.executors.iter().map(|ports| ports.testing).collect(),
            prometheus_port: ports.prometheus,
        }
    }

    async fn fail(&mut self, reason: &str) {
        error!(
            reason = reason,
            namespace = %self.namespace,
            release = %self.release,
            "k8s stack failure; collecting diagnostics"
        );
        dump_namespace_logs(&self.client, &self.namespace).await;
        if let Some(guard) = self.cleanup.take() {
            Box::new(guard).cleanup();
        }
    }

    fn into_cleanup(mut self) -> RunnerCleanup {
        self.cleanup
            .take()
            .expect("cleanup guard should be available")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NodeClientError {
    #[error("failed to build {endpoint} client URL for {role} port {port}: {source}")]
    Endpoint {
        role: NodeRole,
        endpoint: &'static str,
        port: u16,
        #[source]
        source: ParseError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum RemoteReadinessError {
    #[error("failed to build readiness URL for {role} port {port}: {source}")]
    Endpoint {
        role: NodeRole,
        port: u16,
        #[source]
        source: ParseError,
    },
    #[error("remote readiness probe failed: {source}")]
    Remote {
        #[source]
        source: ReadinessError,
    },
}

fn readiness_urls(ports: &[u16], role: NodeRole) -> Result<Vec<Url>, RemoteReadinessError> {
    ports
        .iter()
        .copied()
        .map(|port| readiness_url(role, port))
        .collect()
}

fn readiness_url(role: NodeRole, port: u16) -> Result<Url, RemoteReadinessError> {
    cluster_host_url(port).map_err(|source| RemoteReadinessError::Endpoint { role, port, source })
}

fn cluster_host_url(port: u16) -> Result<Url, ParseError> {
    Url::parse(&format!("http://{}:{port}/", node_host()))
}

fn metrics_handle_from_port(port: u16) -> Result<Metrics, MetricsError> {
    let url = cluster_host_url(port)
        .map_err(|err| MetricsError::new(format!("invalid prometheus url: {err}")))?;
    Metrics::from_prometheus(url)
}

async fn spawn_block_feed_with(
    node_clients: &NodeClients,
) -> Result<(BlockFeed, BlockFeedTask), K8sRunnerError> {
    let block_source_client = node_clients
        .any_client()
        .cloned()
        .ok_or(K8sRunnerError::BlockFeedMissing)?;

    spawn_block_feed(block_source_client)
        .await
        .map_err(|source| K8sRunnerError::BlockFeed { source })
}

#[derive(Debug, thiserror::Error)]
pub enum K8sRunnerError {
    #[error(
        "kubernetes runner requires at least one validator and one executor (validators={validators}, executors={executors})"
    )]
    UnsupportedTopology { validators: usize, executors: usize },
    #[error("failed to initialise kubernetes client: {source}")]
    ClientInit {
        #[source]
        source: kube::Error,
    },
    #[error(transparent)]
    Assets(#[from] AssetsError),
    #[error(transparent)]
    Helm(#[from] HelmError),
    #[error(transparent)]
    Cluster(#[from] Box<ClusterWaitError>),
    #[error(transparent)]
    Readiness(#[from] RemoteReadinessError),
    #[error(transparent)]
    NodeClients(#[from] NodeClientError),
    #[error(transparent)]
    Telemetry(#[from] MetricsError),
    #[error("k8s runner requires at least one node client to follow blocks")]
    BlockFeedMissing,
    #[error("failed to initialize block feed: {source}")]
    BlockFeed {
        #[source]
        source: Error,
    },
}

#[async_trait]
impl Deployer for K8sRunner {
    type Error = K8sRunnerError;

    async fn deploy(&self, scenario: &Scenario) -> Result<Runner, Self::Error> {
        let descriptors = scenario.topology().clone();
        ensure_supported_topology(&descriptors)?;

        let client = Client::try_default()
            .await
            .map_err(|source| K8sRunnerError::ClientInit { source })?;
        info!(
            validators = descriptors.validators().len(),
            executors = descriptors.executors().len(),
            "starting k8s deployment"
        );

        let port_specs = collect_port_specs(&descriptors);
        let mut cluster =
            Some(setup_cluster(&client, &port_specs, &descriptors, self.readiness_checks).await?);

        info!("building node clients");
        let node_clients = match build_node_clients(
            cluster
                .as_ref()
                .expect("cluster must be available while building clients"),
        ) {
            Ok(clients) => clients,
            Err(err) => {
                if let Some(env) = cluster.as_mut() {
                    env.fail("failed to construct node api clients").await;
                }
                return Err(err.into());
            }
        };

        let telemetry = match metrics_handle_from_port(
            cluster
                .as_ref()
                .expect("cluster must be available for telemetry")
                .prometheus_port,
        ) {
            Ok(handle) => handle,
            Err(err) => {
                if let Some(env) = cluster.as_mut() {
                    env.fail("failed to configure prometheus metrics handle")
                        .await;
                }
                return Err(err.into());
            }
        };
        let (block_feed, block_feed_guard) = match spawn_block_feed_with(&node_clients).await {
            Ok(pair) => pair,
            Err(err) => {
                if let Some(env) = cluster.as_mut() {
                    env.fail("failed to initialize block feed").await;
                }
                return Err(err);
            }
        };
        let cleanup = cluster
            .take()
            .expect("cluster should still be available")
            .into_cleanup();
        let cleanup_guard: Box<dyn CleanupGuard> =
            Box::new(K8sCleanupGuard::new(cleanup, block_feed_guard));
        let context = RunContext::new(
            descriptors,
            None,
            node_clients,
            scenario.duration(),
            telemetry,
            block_feed,
        );
        Ok(Runner::new(context, Some(cleanup_guard)))
    }
}

impl From<ClusterWaitError> for K8sRunnerError {
    fn from(value: ClusterWaitError) -> Self {
        Self::Cluster(Box::new(value))
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

fn build_node_clients(cluster: &ClusterEnvironment) -> Result<NodeClients, NodeClientError> {
    let validators = cluster
        .validator_api_ports
        .iter()
        .copied()
        .zip(cluster.validator_testing_ports.iter().copied())
        .map(|(api_port, testing_port)| {
            api_client_from_ports(NodeRole::Validator, api_port, testing_port)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let executors = cluster
        .executor_api_ports
        .iter()
        .copied()
        .zip(cluster.executor_testing_ports.iter().copied())
        .map(|(api_port, testing_port)| {
            api_client_from_ports(NodeRole::Executor, api_port, testing_port)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(NodeClients::new(validators, executors))
}

fn api_client_from_ports(
    role: NodeRole,
    api_port: u16,
    testing_port: u16,
) -> Result<ApiClient, NodeClientError> {
    let base_endpoint = cluster_host_url(api_port).map_err(|source| NodeClientError::Endpoint {
        role,
        endpoint: "api",
        port: api_port,
        source,
    })?;
    let testing_endpoint =
        Some(
            cluster_host_url(testing_port).map_err(|source| NodeClientError::Endpoint {
                role,
                endpoint: "testing",
                port: testing_port,
                source,
            })?,
        );
    Ok(ApiClient::from_urls(base_endpoint, testing_endpoint))
}

async fn setup_cluster(
    client: &Client,
    specs: &PortSpecs,
    descriptors: &GeneratedTopology,
    readiness_checks: bool,
) -> Result<ClusterEnvironment, K8sRunnerError> {
    let assets = prepare_assets(descriptors)?;
    let validators = descriptors.validators().len();
    let executors = descriptors.executors().len();

    let (namespace, release) = cluster_identifiers();

    let mut cleanup_guard =
        Some(install_stack(client, &assets, &namespace, &release, validators, executors).await?);

    let cluster_ports =
        wait_for_ports_or_cleanup(client, &namespace, &release, specs, &mut cleanup_guard).await?;

    info!(
        prometheus_port = cluster_ports.prometheus,
        "discovered prometheus endpoint"
    );

    let environment = ClusterEnvironment::new(
        client.clone(),
        namespace,
        release,
        cleanup_guard
            .take()
            .expect("cleanup guard must exist after successful cluster startup"),
        &cluster_ports,
    );

    if readiness_checks {
        ensure_cluster_readiness(descriptors, &environment).await?;
    }

    Ok(environment)
}

fn cluster_identifiers() -> (String, String) {
    let run_id = Uuid::new_v4().simple().to_string();
    let namespace = format!("nomos-k8s-{run_id}");
    (namespace.clone(), namespace)
}

async fn install_stack(
    client: &Client,
    assets: &RunnerAssets,
    namespace: &str,
    release: &str,
    validators: usize,
    executors: usize,
) -> Result<RunnerCleanup, K8sRunnerError> {
    info!(
        release = %release,
        namespace = %namespace,
        "installing helm release"
    );
    install_release(assets, release, namespace, validators, executors).await?;
    info!(release = %release, "helm install succeeded");

    let preserve = env::var("K8S_RUNNER_PRESERVE").is_ok();
    Ok(RunnerCleanup::new(
        client.clone(),
        namespace.to_owned(),
        release.to_owned(),
        preserve,
    ))
}

async fn wait_for_ports_or_cleanup(
    client: &Client,
    namespace: &str,
    release: &str,
    specs: &PortSpecs,
    cleanup_guard: &mut Option<RunnerCleanup>,
) -> Result<ClusterPorts, K8sRunnerError> {
    match wait_for_cluster_ready(
        client,
        namespace,
        release,
        &specs.validators,
        &specs.executors,
    )
    .await
    {
        Ok(ports) => Ok(ports),
        Err(err) => {
            cleanup_pending(client, namespace, cleanup_guard).await;
            Err(err.into())
        }
    }
}

async fn cleanup_pending(client: &Client, namespace: &str, guard: &mut Option<RunnerCleanup>) {
    dump_namespace_logs(client, namespace).await;
    if let Some(guard) = guard.take() {
        Box::new(guard).cleanup();
    }
}

async fn ensure_cluster_readiness(
    descriptors: &GeneratedTopology,
    cluster: &ClusterEnvironment,
) -> Result<(), RemoteReadinessError> {
    let validator_urls = readiness_urls(&cluster.validator_api_ports, NodeRole::Validator)?;
    let executor_urls = readiness_urls(&cluster.executor_api_ports, NodeRole::Executor)?;
    let validator_membership_urls =
        readiness_urls(&cluster.validator_testing_ports, NodeRole::Validator)?;
    let executor_membership_urls =
        readiness_urls(&cluster.executor_testing_ports, NodeRole::Executor)?;

    descriptors
        .wait_remote_readiness(
            &validator_urls,
            &executor_urls,
            Some(&validator_membership_urls),
            Some(&executor_membership_urls),
        )
        .await
        .map_err(|source| RemoteReadinessError::Remote { source })
}

struct K8sCleanupGuard {
    cleanup: RunnerCleanup,
    block_feed: Option<BlockFeedTask>,
}

impl K8sCleanupGuard {
    const fn new(cleanup: RunnerCleanup, block_feed: BlockFeedTask) -> Self {
        Self {
            cleanup,
            block_feed: Some(block_feed),
        }
    }
}

impl CleanupGuard for K8sCleanupGuard {
    fn cleanup(mut self: Box<Self>) {
        if let Some(block_feed) = self.block_feed.take() {
            CleanupGuard::cleanup(Box::new(block_feed));
        }
        CleanupGuard::cleanup(Box::new(self.cleanup));
    }
}
