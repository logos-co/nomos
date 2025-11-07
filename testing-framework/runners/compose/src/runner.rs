use std::{
    env,
    net::{Ipv4Addr, TcpListener as StdTcpListener},
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use async_trait::async_trait;
use reqwest::Url;
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{
        BlockFeed, BlockFeedTask, CleanupGuard, Deployer, Metrics, MetricsError, NodeClients,
        RunContext, Runner, Scenario,
        http_probe::{HttpReadinessError, NodeRole},
        spawn_block_feed,
    },
    topology::{GeneratedNodeConfig, GeneratedTopology, ReadinessError},
};
use tracing::{error, info};
use url::ParseError;
use uuid::Uuid;

use crate::{
    cfgsync::{CfgsyncServerHandle, start_cfgsync_server, update_cfgsync_config},
    cleanup::RunnerCleanup,
    compose::{
        ComposeCommandError, ComposeDescriptor, DescriptorBuildError, TemplateError, compose_up,
        dump_compose_logs, write_compose_file,
    },
    wait::{wait_for_executors, wait_for_validators},
    workspace::ComposeWorkspace,
};

pub struct ComposeRunner {
    readiness_checks: bool,
}

impl Default for ComposeRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ComposeRunner {
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

const PROMETHEUS_PORT_ENV: &str = "TEST_FRAMEWORK_PROMETHEUS_PORT";

#[derive(Debug, thiserror::Error)]
pub enum ComposeRunnerError {
    #[error(
        "compose runner requires at least one validator (validators={validators}, executors={executors})"
    )]
    MissingValidator { validators: usize, executors: usize },
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Compose(#[from] ComposeCommandError),
    #[error(transparent)]
    Readiness(#[from] StackReadinessError),
    #[error(transparent)]
    NodeClients(#[from] NodeClientError),
    #[error(transparent)]
    Telemetry(#[from] MetricsError),
    #[error("block feed requires at least one validator client")]
    BlockFeedMissing,
    #[error("failed to start block feed: {source}")]
    BlockFeed {
        #[source]
        source: anyhow::Error,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("failed to prepare compose workspace: {source}")]
pub struct WorkspaceError {
    #[source]
    source: anyhow::Error,
}

impl WorkspaceError {
    const fn new(source: anyhow::Error) -> Self {
        Self { source }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to update cfgsync configuration at {path}: {source}")]
    Cfgsync {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to allocate cfgsync port: {source}")]
    Port {
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to start cfgsync server on port {port}: {source}")]
    CfgsyncStart {
        port: u16,
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to build compose descriptor: {source}")]
    Descriptor {
        #[source]
        source: DescriptorBuildError,
    },
    #[error("failed to render compose template: {source}")]
    Template {
        #[source]
        source: TemplateError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum StackReadinessError {
    #[error(transparent)]
    Http(#[from] HttpReadinessError),
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

#[async_trait]
impl Deployer for ComposeRunner {
    type Error = ComposeRunnerError;

    async fn deploy(&self, scenario: &Scenario) -> Result<Runner, Self::Error> {
        let descriptors = scenario.topology().clone();
        ensure_supported_topology(&descriptors)?;

        info!(
            validators = descriptors.validators().len(),
            executors = descriptors.executors().len(),
            "starting compose deployment"
        );

        let prometheus_port = desired_prometheus_port();
        let mut environment = prepare_environment(&descriptors, prometheus_port).await?;

        info!("waiting for validator HTTP endpoints");
        if let Err(err) = ensure_validators_ready(&descriptors).await {
            environment.fail("validator readiness failed").await;
            return Err(err.into());
        }

        info!("waiting for executor HTTP endpoints");
        if let Err(err) = ensure_executors_ready(&descriptors).await {
            environment.fail("executor readiness failed").await;
            return Err(err.into());
        }

        if self.readiness_checks {
            info!("waiting for remote service readiness");
            if let Err(err) = ensure_remote_readiness(&descriptors).await {
                environment.fail("remote readiness probe failed").await;
                return Err(err.into());
            }
        }

        info!("compose stack ready; building node clients");
        let node_clients = match build_node_clients(&descriptors) {
            Ok(clients) => clients,
            Err(err) => {
                environment
                    .fail("failed to construct node api clients")
                    .await;
                return Err(err.into());
            }
        };
        let telemetry = metrics_handle_from_port(prometheus_port)?;
        let (block_feed, block_feed_guard) = match spawn_block_feed_with(&node_clients).await {
            Ok(pair) => pair,
            Err(err) => {
                environment.fail("failed to initialize block feed").await;
                return Err(err);
            }
        };
        let cleanup_guard: Box<dyn CleanupGuard> = Box::new(ComposeCleanupGuard::new(
            environment.into_cleanup(),
            block_feed_guard,
        ));
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

fn desired_prometheus_port() -> u16 {
    env::var(PROMETHEUS_PORT_ENV)
        .ok()
        .and_then(|raw| raw.parse::<u16>().ok())
        .unwrap_or(9090)
}

fn build_node_clients(descriptors: &GeneratedTopology) -> Result<NodeClients, NodeClientError> {
    let validators = descriptors
        .validators()
        .iter()
        .map(|node| api_client_from_descriptor(node, NodeRole::Validator))
        .collect::<Result<Vec<_>, _>>()?;
    let executors = descriptors
        .executors()
        .iter()
        .map(|node| api_client_from_descriptor(node, NodeRole::Executor))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(NodeClients::new(validators, executors))
}

fn api_client_from_descriptor(
    node: &GeneratedNodeConfig,
    role: NodeRole,
) -> Result<ApiClient, NodeClientError> {
    let api_port = node.api_port();
    let base_url = localhost_url(api_port).map_err(|source| NodeClientError::Endpoint {
        role,
        endpoint: "api",
        port: api_port,
        source,
    })?;

    let testing_port = node.testing_http_port();
    let testing_url =
        Some(
            localhost_url(testing_port).map_err(|source| NodeClientError::Endpoint {
                role,
                endpoint: "testing",
                port: testing_port,
                source,
            })?,
        );

    Ok(ApiClient::from_urls(base_url, testing_url))
}

async fn spawn_block_feed_with(
    node_clients: &NodeClients,
) -> Result<(BlockFeed, BlockFeedTask), ComposeRunnerError> {
    let block_source_client = node_clients
        .any_client()
        .cloned()
        .ok_or(ComposeRunnerError::BlockFeedMissing)?;

    spawn_block_feed(block_source_client)
        .await
        .map_err(|source| ComposeRunnerError::BlockFeed { source })
}

fn localhost_url(port: u16) -> Result<Url, ParseError> {
    Url::parse(&format!("http://127.0.0.1:{port}/"))
}

fn metrics_handle_from_port(port: u16) -> Result<Metrics, MetricsError> {
    let url = localhost_url(port)
        .map_err(|err| MetricsError::new(format!("invalid prometheus url: {err}")))?;
    Metrics::from_prometheus(url)
}

async fn ensure_validators_ready(
    descriptors: &GeneratedTopology,
) -> Result<(), StackReadinessError> {
    let validator_ports = collect_validator_ports(descriptors);
    wait_for_validators(&validator_ports)
        .await
        .map_err(Into::into)
}

async fn ensure_remote_readiness(
    descriptors: &GeneratedTopology,
) -> Result<(), StackReadinessError> {
    let (validator_urls, executor_urls) = readiness_urls(descriptors)?;
    descriptors
        .wait_remote_readiness(&validator_urls, &executor_urls, None, None)
        .await
        .map_err(|source| StackReadinessError::Remote { source })
}

async fn ensure_executors_ready(
    descriptors: &GeneratedTopology,
) -> Result<(), StackReadinessError> {
    let executor_ports = collect_executor_ports(descriptors);
    if executor_ports.is_empty() {
        return Ok(());
    }

    wait_for_executors(&executor_ports)
        .await
        .map_err(Into::into)
}

fn readiness_urls(
    descriptors: &GeneratedTopology,
) -> Result<(Vec<Url>, Vec<Url>), StackReadinessError> {
    let validator_urls = descriptors
        .validators()
        .iter()
        .map(|node| readiness_url(NodeRole::Validator, node.api_port()))
        .collect::<Result<Vec<_>, _>>()?;
    let executor_urls = descriptors
        .executors()
        .iter()
        .map(|node| readiness_url(NodeRole::Executor, node.api_port()))
        .collect::<Result<Vec<_>, _>>()?;

    Ok((validator_urls, executor_urls))
}

fn readiness_url(role: NodeRole, port: u16) -> Result<Url, StackReadinessError> {
    localhost_url(port).map_err(|source| StackReadinessError::Endpoint { role, port, source })
}

fn collect_validator_ports(descriptors: &GeneratedTopology) -> Vec<u16> {
    descriptors
        .validators()
        .iter()
        .map(GeneratedNodeConfig::api_port)
        .collect()
}

fn collect_executor_ports(descriptors: &GeneratedTopology) -> Vec<u16> {
    descriptors
        .executors()
        .iter()
        .map(GeneratedNodeConfig::api_port)
        .collect()
}

struct WorkspaceState {
    workspace: ComposeWorkspace,
    root: PathBuf,
    cfgsync_path: PathBuf,
    use_kzg: bool,
}

fn ensure_supported_topology(descriptors: &GeneratedTopology) -> Result<(), ComposeRunnerError> {
    let validators = descriptors.validators().len();
    if validators == 0 {
        return Err(ComposeRunnerError::MissingValidator {
            validators,
            executors: descriptors.executors().len(),
        });
    }
    Ok(())
}

async fn prepare_environment(
    descriptors: &GeneratedTopology,
    prometheus_port: u16,
) -> Result<StackEnvironment, ComposeRunnerError> {
    let workspace = prepare_workspace_logged()?;
    update_cfgsync_logged(&workspace, descriptors)?;

    let (cfgsync_port, mut cfgsync_handle) = start_cfgsync_stage(&workspace).await?;
    let compose_path =
        render_compose_logged(&workspace, descriptors, cfgsync_port, prometheus_port)?;

    let project_name = format!("nomos-compose-{}", Uuid::new_v4());
    bring_up_stack_logged(
        &compose_path,
        &project_name,
        &workspace.root,
        &mut cfgsync_handle,
    )
    .await?;

    Ok(StackEnvironment::from_workspace(
        workspace,
        compose_path,
        project_name,
        Some(cfgsync_handle),
    ))
}

fn prepare_workspace_state() -> Result<WorkspaceState, WorkspaceError> {
    let workspace = ComposeWorkspace::create().map_err(WorkspaceError::new)?;
    let root = workspace.root_path().to_path_buf();
    let cfgsync_path = workspace.testnet_dir().join("cfgsync.yaml");
    let use_kzg = workspace.root_path().join("kzgrs_test_params").exists();

    Ok(WorkspaceState {
        workspace,
        root,
        cfgsync_path,
        use_kzg,
    })
}

fn prepare_workspace_logged() -> Result<WorkspaceState, ComposeRunnerError> {
    info!("preparing compose workspace");
    prepare_workspace_state().map_err(Into::into)
}

fn update_cfgsync_logged(
    workspace: &WorkspaceState,
    descriptors: &GeneratedTopology,
) -> Result<(), ComposeRunnerError> {
    info!("updating cfgsync configuration");
    configure_cfgsync(workspace, descriptors).map_err(Into::into)
}

async fn start_cfgsync_stage(
    workspace: &WorkspaceState,
) -> Result<(u16, CfgsyncServerHandle), ComposeRunnerError> {
    let cfgsync_port = allocate_cfgsync_port()?;
    info!(cfgsync_port = cfgsync_port, "launching cfgsync server");
    let handle = launch_cfgsync(&workspace.cfgsync_path, cfgsync_port).await?;
    Ok((cfgsync_port, handle))
}

fn configure_cfgsync(
    workspace: &WorkspaceState,
    descriptors: &GeneratedTopology,
) -> Result<(), ConfigError> {
    update_cfgsync_config(&workspace.cfgsync_path, descriptors, workspace.use_kzg).map_err(
        |source| ConfigError::Cfgsync {
            path: workspace.cfgsync_path.clone(),
            source,
        },
    )
}

fn allocate_cfgsync_port() -> Result<u16, ConfigError> {
    let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .context("allocating cfgsync port")
        .map_err(|source| ConfigError::Port { source })?;

    let port = listener
        .local_addr()
        .context("reading cfgsync port")
        .map_err(|source| ConfigError::Port { source })?
        .port();
    Ok(port)
}

async fn launch_cfgsync(
    cfgsync_path: &Path,
    port: u16,
) -> Result<CfgsyncServerHandle, ConfigError> {
    start_cfgsync_server(cfgsync_path, port)
        .await
        .map_err(|source| ConfigError::CfgsyncStart { port, source })
}

fn write_compose_artifacts(
    workspace: &WorkspaceState,
    descriptors: &GeneratedTopology,
    cfgsync_port: u16,
    prometheus_port: u16,
) -> Result<PathBuf, ConfigError> {
    let descriptor = ComposeDescriptor::builder(descriptors)
        .with_kzg_mount(workspace.use_kzg)
        .with_cfgsync_port(cfgsync_port)
        .with_prometheus_port(prometheus_port)
        .build()
        .map_err(|source| ConfigError::Descriptor { source })?;

    let compose_path = workspace.root.join("compose.generated.yml");
    write_compose_file(&descriptor, &compose_path)
        .map_err(|source| ConfigError::Template { source })?;
    Ok(compose_path)
}

fn render_compose_logged(
    workspace: &WorkspaceState,
    descriptors: &GeneratedTopology,
    cfgsync_port: u16,
    prometheus_port: u16,
) -> Result<PathBuf, ComposeRunnerError> {
    info!("rendering compose file");
    write_compose_artifacts(workspace, descriptors, cfgsync_port, prometheus_port)
        .map_err(Into::into)
}

async fn bring_up_stack(
    compose_path: &Path,
    project_name: &str,
    workspace_root: &Path,
    cfgsync_handle: &mut CfgsyncServerHandle,
) -> Result<(), ComposeRunnerError> {
    if let Err(err) = compose_up(compose_path, project_name, workspace_root).await {
        cfgsync_handle.shutdown();
        return Err(ComposeRunnerError::Compose(err));
    }
    Ok(())
}

async fn bring_up_stack_logged(
    compose_path: &Path,
    project_name: &str,
    workspace_root: &Path,
    cfgsync_handle: &mut CfgsyncServerHandle,
) -> Result<(), ComposeRunnerError> {
    info!(project = %project_name, "bringing up docker compose stack");
    bring_up_stack(compose_path, project_name, workspace_root, cfgsync_handle).await
}

struct StackEnvironment {
    compose_path: PathBuf,
    project_name: String,
    root: PathBuf,
    workspace: Option<ComposeWorkspace>,
    cfgsync_handle: Option<CfgsyncServerHandle>,
}

impl StackEnvironment {
    fn from_workspace(
        state: WorkspaceState,
        compose_path: PathBuf,
        project_name: String,
        cfgsync_handle: Option<CfgsyncServerHandle>,
    ) -> Self {
        let WorkspaceState {
            workspace, root, ..
        } = state;

        Self {
            compose_path,
            project_name,
            root,
            workspace: Some(workspace),
            cfgsync_handle,
        }
    }

    fn compose_path(&self) -> &Path {
        &self.compose_path
    }

    fn project_name(&self) -> &str {
        &self.project_name
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn take_cleanup(&mut self) -> RunnerCleanup {
        RunnerCleanup::new(
            self.compose_path.clone(),
            self.project_name.clone(),
            self.root.clone(),
            self.workspace
                .take()
                .expect("workspace must be available while cleaning up"),
            self.cfgsync_handle.take(),
        )
    }

    fn into_cleanup(self) -> RunnerCleanup {
        RunnerCleanup::new(
            self.compose_path,
            self.project_name,
            self.root,
            self.workspace
                .expect("workspace must be available while cleaning up"),
            self.cfgsync_handle,
        )
    }

    async fn fail(&mut self, reason: &str) {
        error!(
            reason = reason,
            "compose stack failure; dumping docker logs"
        );
        dump_compose_logs(self.compose_path(), self.project_name(), self.root()).await;
        Box::new(self.take_cleanup()).cleanup();
    }
}

struct ComposeCleanupGuard {
    environment: RunnerCleanup,
    block_feed: Option<BlockFeedTask>,
}

impl ComposeCleanupGuard {
    const fn new(environment: RunnerCleanup, block_feed: BlockFeedTask) -> Self {
        Self {
            environment,
            block_feed: Some(block_feed),
        }
    }
}

impl CleanupGuard for ComposeCleanupGuard {
    fn cleanup(mut self: Box<Self>) {
        if let Some(block_feed) = self.block_feed.take() {
            CleanupGuard::cleanup(Box::new(block_feed));
        }
        CleanupGuard::cleanup(Box::new(self.environment));
    }
}
