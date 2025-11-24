use std::{
    collections::HashMap,
    env,
    net::{Ipv4Addr, TcpListener as StdTcpListener},
    path::{Path, PathBuf},
    process::{Command as StdCommand, Stdio},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context as _, anyhow};
use async_trait::async_trait;
use reqwest::Url;
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{
        BlockFeed, BlockFeedTask, CleanupGuard, Deployer, DynError, Metrics, MetricsError,
        NodeClients, NodeControlHandle, RequiresNodeControl, RunContext, Runner, Scenario,
        http_probe::{HttpReadinessError, NodeRole as HttpNodeRole},
        spawn_block_feed,
    },
    topology::{
        GeneratedTopology, NodeRole as TopologyNodeRole, ReadinessError,
        configs::GeneralConfig as IntegrationGeneralConfig,
    },
};
use tokio::{
    process::Command,
    time::{sleep, timeout},
};
use tracing::{error, info, warn};
use url::ParseError;
use uuid::Uuid;

use crate::{
    cfgsync::{CfgsyncServerHandle, start_cfgsync_server, update_cfgsync_config},
    cleanup::RunnerCleanup,
    compose::{
        ComposeCommandError, ComposeDescriptor, DescriptorBuildError, HostPortMapping,
        NodeHostPorts, TemplateError, compose_up, dump_compose_logs, repository_root,
        resolve_image, write_compose_file,
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
const DEFAULT_PROMETHEUS_PORT: u16 = 9090;
const IMAGE_BUILD_TIMEOUT: Duration = Duration::from_secs(600);
const BLOCK_FEED_MAX_ATTEMPTS: usize = 5;
const BLOCK_FEED_RETRY_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum ComposeRunnerError {
    #[error(
        "compose runner requires at least one validator (validators={validators}, executors={executors})"
    )]
    MissingValidator { validators: usize, executors: usize },
    #[error("docker does not appear to be available on this host")]
    DockerUnavailable,
    #[error("failed to resolve host port for {service} container port {container_port}: {source}")]
    PortDiscovery {
        service: String,
        container_port: u16,
        #[source]
        source: anyhow::Error,
    },
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
    #[error(
        "docker image '{image}' is not available; set NOMOS_TESTNET_IMAGE or build the image manually"
    )]
    MissingImage { image: String },
    #[error("failed to prepare docker image: {source}")]
    ImageBuild {
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
        role: HttpNodeRole,
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
        role: HttpNodeRole,
        endpoint: &'static str,
        port: u16,
        #[source]
        source: ParseError,
    },
}

#[async_trait]
impl<Caps> Deployer<Caps> for ComposeRunner
where
    Caps: RequiresNodeControl + Send + Sync,
{
    type Error = ComposeRunnerError;

    async fn deploy(&self, scenario: &Scenario<Caps>) -> Result<Runner, Self::Error> {
        ensure_docker_available()?;
        let descriptors = scenario.topology().clone();
        ensure_supported_topology(&descriptors)?;

        info!(
            validators = descriptors.validators().len(),
            executors = descriptors.executors().len(),
            "starting compose deployment"
        );

        let prometheus_port = desired_prometheus_port();
        let mut environment = prepare_environment(&descriptors, prometheus_port).await?;

        let host_ports = match discover_host_ports(&environment, &descriptors).await {
            Ok(mapping) => mapping,
            Err(err) => {
                environment
                    .fail("failed to determine container host ports")
                    .await;
                return Err(err);
            }
        };

        if self.readiness_checks {
            info!("waiting for validator HTTP endpoints");
            if let Err(err) =
                ensure_validators_ready_with_ports(&host_ports.validator_api_ports()).await
            {
                environment.fail("validator readiness failed").await;
                return Err(err.into());
            }

            info!("waiting for executor HTTP endpoints");
            if let Err(err) =
                ensure_executors_ready_with_ports(&host_ports.executor_api_ports()).await
            {
                environment.fail("executor readiness failed").await;
                return Err(err.into());
            }

            info!("waiting for remote service readiness");
            if let Err(err) = ensure_remote_readiness_with_ports(&descriptors, &host_ports).await {
                environment.fail("remote readiness probe failed").await;
                return Err(err.into());
            }
        } else {
            info!("readiness checks disabled; giving the stack a short grace period");
            sleep(Duration::from_secs(5)).await;
        }

        info!("compose stack ready; building node clients");
        let node_clients = match build_node_clients_with_ports(&descriptors, &host_ports) {
            Ok(clients) => clients,
            Err(err) => {
                environment
                    .fail("failed to construct node api clients")
                    .await;
                return Err(err.into());
            }
        };
        let telemetry = metrics_handle_from_port(prometheus_port)?;
        let node_control = Caps::REQUIRED.then(|| {
            Arc::new(ComposeNodeControl::new(
                environment.compose_path().to_path_buf(),
                environment.project_name().to_owned(),
            )) as Arc<dyn NodeControlHandle>
        });
        let (block_feed, block_feed_guard) = match spawn_block_feed_with_retry(&node_clients).await
        {
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
            node_control,
        );

        Ok(Runner::new(context, Some(cleanup_guard)))
    }
}

fn desired_prometheus_port() -> u16 {
    env::var(PROMETHEUS_PORT_ENV)
        .ok()
        .and_then(|raw| raw.parse::<u16>().ok())
        .unwrap_or_else(|| allocate_prometheus_port().unwrap_or(DEFAULT_PROMETHEUS_PORT))
}

fn allocate_prometheus_port() -> Option<u16> {
    let try_bind = |port| StdTcpListener::bind((Ipv4Addr::LOCALHOST, port));
    let listener = try_bind(DEFAULT_PROMETHEUS_PORT)
        .or_else(|_| try_bind(0))
        .ok()?;
    listener.local_addr().ok().map(|addr| addr.port())
}

fn build_node_clients_with_ports(
    descriptors: &GeneratedTopology,
    mapping: &HostPortMapping,
) -> Result<NodeClients, NodeClientError> {
    let validators = descriptors
        .validators()
        .iter()
        .zip(mapping.validators.iter())
        .map(|(node, ports)| api_client_from_host_ports(to_http_role(node.role()), ports))
        .collect::<Result<Vec<_>, _>>()?;
    let executors = descriptors
        .executors()
        .iter()
        .zip(mapping.executors.iter())
        .map(|(node, ports)| api_client_from_host_ports(to_http_role(node.role()), ports))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(NodeClients::new(validators, executors))
}

fn api_client_from_host_ports(
    role: HttpNodeRole,
    ports: &NodeHostPorts,
) -> Result<ApiClient, NodeClientError> {
    let base_url = localhost_url(ports.api).map_err(|source| NodeClientError::Endpoint {
        role,
        endpoint: "api",
        port: ports.api,
        source,
    })?;

    let testing_url =
        Some(
            localhost_url(ports.testing).map_err(|source| NodeClientError::Endpoint {
                role,
                endpoint: "testing",
                port: ports.testing,
                source,
            })?,
        );

    Ok(ApiClient::from_urls(base_url, testing_url))
}

const fn to_http_role(role: TopologyNodeRole) -> HttpNodeRole {
    match role {
        TopologyNodeRole::Validator => HttpNodeRole::Validator,
        TopologyNodeRole::Executor => HttpNodeRole::Executor,
    }
}

async fn spawn_block_feed_with(
    node_clients: &NodeClients,
) -> Result<(BlockFeed, BlockFeedTask), ComposeRunnerError> {
    let block_source_client = node_clients
        .random_validator()
        .cloned()
        .ok_or(ComposeRunnerError::BlockFeedMissing)?;

    spawn_block_feed(block_source_client)
        .await
        .map_err(|source| ComposeRunnerError::BlockFeed { source })
}

async fn spawn_block_feed_with_retry(
    node_clients: &NodeClients,
) -> Result<(BlockFeed, BlockFeedTask), ComposeRunnerError> {
    let mut last_err = None;
    for attempt in 1..=BLOCK_FEED_MAX_ATTEMPTS {
        match spawn_block_feed_with(node_clients).await {
            Ok(result) => return Ok(result),
            Err(err) => {
                last_err = Some(err);
                if attempt < BLOCK_FEED_MAX_ATTEMPTS {
                    warn!(attempt, "block feed initialization failed; retrying");
                    sleep(BLOCK_FEED_RETRY_DELAY).await;
                }
            }
        }
    }

    Err(last_err.expect("block feed retry should capture an error"))
}

async fn restart_compose_service(
    compose_file: &Path,
    project_name: &str,
    service: &str,
) -> Result<(), ComposeRunnerError> {
    let mut command = Command::new("docker");
    command
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("-p")
        .arg(project_name)
        .arg("restart")
        .arg(service);

    let description = "docker compose restart";
    run_docker_command(command, description, Duration::from_secs(120)).await
}

struct ComposeNodeControl {
    compose_file: PathBuf,
    project_name: String,
}

impl ComposeNodeControl {
    #[expect(
        clippy::missing_const_for_fn,
        reason = "PathBuf and String construction is not const on stable"
    )]
    fn new(compose_file: PathBuf, project_name: String) -> Self {
        Self {
            compose_file,
            project_name,
        }
    }
}

#[async_trait]
impl NodeControlHandle for ComposeNodeControl {
    async fn restart_validator(&self, index: usize) -> Result<(), DynError> {
        restart_compose_service(
            &self.compose_file,
            &self.project_name,
            &format!("validator-{index}"),
        )
        .await
        .map_err(|err| format!("validator restart failed: {err}").into())
    }

    async fn restart_executor(&self, index: usize) -> Result<(), DynError> {
        restart_compose_service(
            &self.compose_file,
            &self.project_name,
            &format!("executor-{index}"),
        )
        .await
        .map_err(|err| format!("executor restart failed: {err}").into())
    }
}

fn localhost_url(port: u16) -> Result<Url, ParseError> {
    Url::parse(&format!("http://127.0.0.1:{port}/"))
}

async fn discover_host_ports(
    environment: &StackEnvironment,
    descriptors: &GeneratedTopology,
) -> Result<HostPortMapping, ComposeRunnerError> {
    let mut validators = Vec::new();
    for node in descriptors.validators() {
        let service = node_identifier(TopologyNodeRole::Validator, node.index());
        let api = resolve_service_port(environment, &service, node.api_port()).await?;
        let testing = resolve_service_port(environment, &service, node.testing_http_port()).await?;
        validators.push(NodeHostPorts { api, testing });
    }

    let mut executors = Vec::new();
    for node in descriptors.executors() {
        let service = node_identifier(TopologyNodeRole::Executor, node.index());
        let api = resolve_service_port(environment, &service, node.api_port()).await?;
        let testing = resolve_service_port(environment, &service, node.testing_http_port()).await?;
        executors.push(NodeHostPorts { api, testing });
    }

    Ok(HostPortMapping {
        validators,
        executors,
    })
}

async fn resolve_service_port(
    environment: &StackEnvironment,
    service: &str,
    container_port: u16,
) -> Result<u16, ComposeRunnerError> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(environment.compose_path())
        .arg("-p")
        .arg(environment.project_name())
        .arg("port")
        .arg(service)
        .arg(container_port.to_string())
        .current_dir(environment.root());

    let output = cmd
        .output()
        .await
        .with_context(|| format!("running docker compose port {service} {container_port}"))
        .map_err(|source| ComposeRunnerError::PortDiscovery {
            service: service.to_owned(),
            container_port,
            source,
        })?;

    if !output.status.success() {
        return Err(ComposeRunnerError::PortDiscovery {
            service: service.to_owned(),
            container_port,
            source: anyhow!("docker compose port exited with {}", output.status),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(port_str) = line.rsplit(':').next()
            && let Ok(port) = port_str.trim().parse::<u16>()
        {
            return Ok(port);
        }
    }

    Err(ComposeRunnerError::PortDiscovery {
        service: service.to_owned(),
        container_port,
        source: anyhow!("unable to parse docker compose port output: {stdout}"),
    })
}

fn ensure_docker_available() -> Result<(), ComposeRunnerError> {
    let available = StdCommand::new("docker")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if available {
        Ok(())
    } else {
        Err(ComposeRunnerError::DockerUnavailable)
    }
}

fn metrics_handle_from_port(port: u16) -> Result<Metrics, MetricsError> {
    let url = localhost_url(port)
        .map_err(|err| MetricsError::new(format!("invalid prometheus url: {err}")))?;
    Metrics::from_prometheus(url)
}

async fn ensure_validators_ready_with_ports(ports: &[u16]) -> Result<(), StackReadinessError> {
    if ports.is_empty() {
        return Ok(());
    }

    wait_for_validators(ports).await.map_err(Into::into)
}

async fn ensure_executors_ready_with_ports(ports: &[u16]) -> Result<(), StackReadinessError> {
    if ports.is_empty() {
        return Ok(());
    }

    wait_for_executors(ports).await.map_err(Into::into)
}

async fn ensure_remote_readiness_with_ports(
    descriptors: &GeneratedTopology,
    mapping: &HostPortMapping,
) -> Result<(), StackReadinessError> {
    let validator_urls = mapping
        .validators
        .iter()
        .map(|ports| readiness_url(HttpNodeRole::Validator, ports.api))
        .collect::<Result<Vec<_>, _>>()?;
    let executor_urls = mapping
        .executors
        .iter()
        .map(|ports| readiness_url(HttpNodeRole::Executor, ports.api))
        .collect::<Result<Vec<_>, _>>()?;

    descriptors
        .wait_remote_readiness(&validator_urls, &executor_urls, None, None)
        .await
        .map_err(|source| StackReadinessError::Remote { source })
}

fn readiness_url(role: HttpNodeRole, port: u16) -> Result<Url, StackReadinessError> {
    localhost_url(port).map_err(|source| StackReadinessError::Endpoint { role, port, source })
}

fn collect_prebuilt_configs(
    descriptors: &GeneratedTopology,
) -> HashMap<String, IntegrationGeneralConfig> {
    let mut configs = HashMap::new();
    for node in descriptors.validators() {
        configs.insert(
            node_identifier(TopologyNodeRole::Validator, node.index()),
            node.general.clone(),
        );
    }
    for node in descriptors.executors() {
        configs.insert(
            node_identifier(TopologyNodeRole::Executor, node.index()),
            node.general.clone(),
        );
    }
    configs
}

fn node_identifier(role: TopologyNodeRole, index: usize) -> String {
    match role {
        TopologyNodeRole::Validator => format!("validator-{index}"),
        TopologyNodeRole::Executor => format!("executor-{index}"),
    }
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
    ensure_compose_image().await?;

    let prebuilt_configs = collect_prebuilt_configs(descriptors);
    assert!(
        !prebuilt_configs.is_empty(),
        "compose runner must inject prebuilt configs for cfgsync"
    );
    let (cfgsync_port, mut cfgsync_handle) =
        start_cfgsync_stage(&workspace, prebuilt_configs).await?;
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
    prebuilt_configs: HashMap<String, IntegrationGeneralConfig>,
) -> Result<(u16, CfgsyncServerHandle), ComposeRunnerError> {
    let cfgsync_port = allocate_cfgsync_port()?;
    info!(cfgsync_port = cfgsync_port, "launching cfgsync server");
    let handle = launch_cfgsync(&workspace.cfgsync_path, cfgsync_port, prebuilt_configs).await?;
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
    prebuilt_configs: HashMap<String, IntegrationGeneralConfig>,
) -> Result<CfgsyncServerHandle, ConfigError> {
    start_cfgsync_server(cfgsync_path, port, prebuilt_configs)
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

async fn ensure_compose_image() -> Result<(), ComposeRunnerError> {
    let (image, platform) = resolve_image();
    ensure_image_present(&image, platform.as_deref()).await
}

async fn ensure_image_present(
    image: &str,
    platform: Option<&str>,
) -> Result<(), ComposeRunnerError> {
    if docker_image_exists(image).await? {
        return Ok(());
    }

    if image != "nomos-testnet:local" {
        return Err(ComposeRunnerError::MissingImage {
            image: image.to_owned(),
        });
    }

    build_local_image(image, platform).await
}

async fn docker_image_exists(image: &str) -> Result<bool, ComposeRunnerError> {
    let mut cmd = Command::new("docker");
    cmd.arg("image")
        .arg("inspect")
        .arg(image)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    match cmd.status().await {
        Ok(status) => Ok(status.success()),
        Err(source) => Err(ComposeRunnerError::Compose(ComposeCommandError::Spawn {
            command: format!("docker image inspect {image}"),
            source,
        })),
    }
}

async fn build_local_image(image: &str, platform: Option<&str>) -> Result<(), ComposeRunnerError> {
    let repo_root =
        repository_root().map_err(|source| ComposeRunnerError::ImageBuild { source })?;
    let dockerfile = repo_root.join("testing-framework/runners/docker/runner.Dockerfile");

    info!(image, "building compose runner docker image");

    let mut cmd = Command::new("docker");
    cmd.arg("build");

    if let Some(build_platform) = select_build_platform(platform)? {
        cmd.arg("--platform").arg(&build_platform);
    }

    let circuits_platform = env::var("COMPOSE_CIRCUITS_PLATFORM")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| String::from("linux-x86_64"));

    cmd.arg("--build-arg")
        .arg(format!("NOMOS_CIRCUITS_PLATFORM={circuits_platform}"));

    if let Some(value) = env::var("CIRCUITS_OVERRIDE")
        .ok()
        .filter(|val| !val.is_empty())
    {
        cmd.arg("--build-arg")
            .arg(format!("CIRCUITS_OVERRIDE={value}"));
    }

    cmd.arg("-t")
        .arg(image)
        .arg("-f")
        .arg(&dockerfile)
        .arg(&repo_root);

    run_docker_command(cmd, "docker build compose image", IMAGE_BUILD_TIMEOUT).await
}

async fn run_docker_command(
    mut command: Command,
    description: &str,
    timeout_duration: Duration,
) -> Result<(), ComposeRunnerError> {
    match timeout(timeout_duration, command.status()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(ComposeRunnerError::Compose(ComposeCommandError::Failed {
            command: description.to_owned(),
            status,
        })),
        Ok(Err(source)) => Err(ComposeRunnerError::Compose(ComposeCommandError::Spawn {
            command: description.to_owned(),
            source,
        })),
        Err(_) => Err(ComposeRunnerError::Compose(ComposeCommandError::Timeout {
            command: description.to_owned(),
            timeout: timeout_duration,
        })),
    }
}

fn detect_docker_platform() -> Result<Option<String>, ComposeRunnerError> {
    let output = StdCommand::new("docker")
        .arg("info")
        .arg("-f")
        .arg("{{.Architecture}}")
        .output()
        .map_err(|source| ComposeRunnerError::ImageBuild {
            source: source.into(),
        })?;

    if !output.status.success() {
        return Ok(None);
    }

    let arch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if arch.is_empty() {
        return Ok(None);
    }

    Ok(Some(format!("linux/{arch}")))
}

fn select_build_platform(requested: Option<&str>) -> Result<Option<String>, ComposeRunnerError> {
    if let Some(value) = requested {
        return Ok(Some(value.to_owned()));
    }

    detect_docker_platform()?.map_or_else(
        || {
            warn!("docker host architecture unavailable; letting docker choose default platform");
            Ok(None)
        },
        |host_platform| Ok(Some(host_platform)),
    )
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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, net::Ipv4Addr};

    use cfgsync::config::{Host, HostPorts, create_node_configs};
    use groth16::Fr;
    use nomos_core::{
        mantle::{GenesisTx as GenesisTxTrait, ledger::NoteId},
        sdp::{ProviderId, ServiceType},
    };
    use nomos_ledger::LedgerState;
    use nomos_tracing_service::TracingSettings;
    use testing_framework_core::{
        scenario::ScenarioBuilder,
        topology::{GeneratedNodeConfig, GeneratedTopology, NodeRole as TopologyNodeRole},
    };
    use zksign::PublicKey;

    use super::collect_prebuilt_configs;

    #[test]
    fn cfgsync_prebuilt_configs_preserve_genesis() {
        let scenario = ScenarioBuilder::with_node_counts(1, 1).build();
        let topology = scenario.topology().clone();
        let hosts = hosts_from_topology(&topology);
        let prebuilt = collect_prebuilt_configs(&topology);
        let tracing_settings = tracing_settings(&topology);

        let configs = create_node_configs(
            &topology.config().consensus_params,
            &topology.config().da_params,
            &tracing_settings,
            hosts,
            Some(&prebuilt),
        );
        let configs_by_identifier: HashMap<_, _> = configs
            .into_iter()
            .map(|(host, config)| (host.identifier, config))
            .collect();

        for node in topology.nodes() {
            let identifier = identifier_for(node.role(), node.index());
            let cfgsync_config = configs_by_identifier
                .get(&identifier)
                .unwrap_or_else(|| panic!("missing cfgsync config for {identifier}"));
            let expected_genesis = &node.general.consensus_config.genesis_tx;
            let actual_genesis = &cfgsync_config.consensus_config.genesis_tx;
            if std::env::var("PRINT_GENESIS").is_ok() {
                println!(
                    "[fingerprint {identifier}] expected={:?}",
                    declaration_fingerprint(expected_genesis)
                );
                println!(
                    "[fingerprint {identifier}] actual={:?}",
                    declaration_fingerprint(actual_genesis)
                );
            }
            assert_eq!(
                expected_genesis.mantle_tx().ledger_tx,
                actual_genesis.mantle_tx().ledger_tx,
                "ledger tx mismatch for {identifier}"
            );
            assert_eq!(
                declaration_fingerprint(expected_genesis),
                declaration_fingerprint(actual_genesis),
                "declaration entries mismatch for {identifier}"
            );
        }
    }

    #[test]
    fn cfgsync_genesis_proofs_verify_against_ledger() {
        let scenario = ScenarioBuilder::with_node_counts(1, 1).build();
        let topology = scenario.topology().clone();
        let hosts = hosts_from_topology(&topology);
        let prebuilt = collect_prebuilt_configs(&topology);
        let tracing_settings = tracing_settings(&topology);

        let configs = create_node_configs(
            &topology.config().consensus_params,
            &topology.config().da_params,
            &tracing_settings,
            hosts,
            Some(&prebuilt),
        );
        let configs_by_identifier: HashMap<_, _> = configs
            .into_iter()
            .map(|(host, config)| (host.identifier, config))
            .collect();

        for node in topology.nodes() {
            let identifier = identifier_for(node.role(), node.index());
            let cfgsync_config = configs_by_identifier
                .get(&identifier)
                .unwrap_or_else(|| panic!("missing cfgsync config for {identifier}"));
            LedgerState::from_genesis_tx::<()>(
                cfgsync_config.consensus_config.genesis_tx.clone(),
                &cfgsync_config.consensus_config.ledger_config,
                Fr::from(0u64),
            )
            .unwrap_or_else(|err| panic!("ledger rejected genesis for {identifier}: {err:?}"));
        }
    }

    #[test]
    fn cfgsync_docker_overrides_produce_valid_genesis() {
        let scenario = ScenarioBuilder::with_node_counts(1, 1).build();
        let topology = scenario.topology().clone();
        let prebuilt = collect_prebuilt_configs(&topology);
        let tracing_settings = tracing_settings(&topology);
        let hosts = docker_style_hosts(&topology);

        let configs = create_node_configs(
            &topology.config().consensus_params,
            &topology.config().da_params,
            &tracing_settings,
            hosts,
            Some(&prebuilt),
        );

        for (host, config) in configs {
            let genesis = &config.consensus_config.genesis_tx;
            LedgerState::from_genesis_tx::<()>(
                genesis.clone(),
                &config.consensus_config.ledger_config,
                Fr::from(0u64),
            )
            .unwrap_or_else(|err| {
                panic!("ledger rejected genesis for {}: {err:?}", host.identifier)
            });
        }
    }

    fn hosts_from_topology(topology: &GeneratedTopology) -> Vec<Host> {
        topology.nodes().map(host_from_node).collect()
    }

    fn docker_style_hosts(topology: &GeneratedTopology) -> Vec<Host> {
        topology
            .nodes()
            .map(|node| docker_host(node, 10 + node.index() as u8))
            .collect()
    }

    fn host_from_node(node: &GeneratedNodeConfig) -> Host {
        let ports = HostPorts {
            network_port: node.network_port(),
            da_network_port: node.da_port,
            blend_port: node.blend_port,
            api_port: node.api_port(),
            testing_http_port: node.testing_http_port(),
        };
        let identifier = identifier_for(node.role(), node.index());
        let ip = Ipv4Addr::LOCALHOST;
        match node.role() {
            TopologyNodeRole::Validator => Host::validator_from_ip(ip, identifier, Some(ports)),
            TopologyNodeRole::Executor => Host::executor_from_ip(ip, identifier, Some(ports)),
        }
    }

    fn docker_host(node: &GeneratedNodeConfig, octet: u8) -> Host {
        let ports = HostPorts {
            network_port: node.network_port() + 1000,
            da_network_port: node.da_port + 1000,
            blend_port: node.blend_port + 1000,
            api_port: node.api_port() + 1000,
            testing_http_port: node.testing_http_port() + 1000,
        };
        let identifier = identifier_for(node.role(), node.index());
        let ip = Ipv4Addr::new(172, 23, 0, octet);
        match node.role() {
            TopologyNodeRole::Validator => Host::validator_from_ip(ip, identifier, Some(ports)),
            TopologyNodeRole::Executor => Host::executor_from_ip(ip, identifier, Some(ports)),
        }
    }

    fn tracing_settings(topology: &GeneratedTopology) -> TracingSettings {
        topology
            .validators()
            .first()
            .or_else(|| topology.executors().first())
            .expect("topology must contain at least one node")
            .general
            .tracing_config
            .tracing_settings
            .clone()
    }

    fn identifier_for(role: TopologyNodeRole, index: usize) -> String {
        match role {
            TopologyNodeRole::Validator => format!("validator-{index}"),
            TopologyNodeRole::Executor => format!("executor-{index}"),
        }
    }

    fn declaration_fingerprint<G>(genesis: &G) -> Vec<(ServiceType, ProviderId, NoteId, PublicKey)>
    where
        G: GenesisTxTrait,
    {
        genesis
            .sdp_declarations()
            .map(|(op, _)| (op.service_type, op.provider_id, op.locked_note_id, op.zk_id))
            .collect()
    }
}
