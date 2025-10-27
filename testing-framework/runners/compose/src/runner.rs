//! Docker-based runner that reuses the compose stack to launch scenarios.

use std::{
    env,
    net::{Ipv4Addr, TcpListener as StdTcpListener},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::Context as _;
use futures::FutureExt as _;
use reqwest::Url;
use testing_framework_core::{
    scenario::{
        CleanupGuard, ExecutionPlan, ExpectationError, HttpNodeClient, Metrics, NodeClientHandle,
        RunContext, RunHandle, Runner, WorkloadError,
    },
    topology::{GeneratedNodeConfig, GeneratedTopology},
};
use uuid::Uuid;

use crate::{
    cfgsync::{CfgsyncServerHandle, start_cfgsync_server, update_cfgsync_config},
    cleanup::RunnerCleanup,
    compose::{compose_up, dump_compose_logs, write_compose_file},
    wait::wait_for_validators,
    workspace::ComposeWorkspace,
};

pub struct ComposeRunner;

impl Default for ComposeRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ComposeRunner {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

const PROMETHEUS_URL_ENV: &str = "TEST_FRAMEWORK_PROMETHEUS_URL";
const PROMETHEUS_PORT_ENV: &str = "TEST_FRAMEWORK_PROMETHEUS_PORT";

#[derive(Debug, thiserror::Error)]
pub enum ComposeRunnerError {
    #[error(
        "compose runner requires at least one validator (validators={validators}, executors={executors})"
    )]
    UnsupportedTopology { validators: usize, executors: usize },
    #[error("failed to prepare compose workspace: {0}")]
    Workspace(String),
    #[error("failed to update cfgsync configuration: {0}")]
    Config(String),
    #[error("docker compose command failed: {0}")]
    Compose(String),
    #[error("validator did not become ready: {0}")]
    Probe(String),
    #[error("workload `{name}` failed: {source}")]
    WorkloadFailed { name: String, source: WorkloadError },
    #[error("expectation `{name}` failed: {source}")]
    ExpectationFailed {
        name: String,
        source: ExpectationError,
    },
}

impl Runner for ComposeRunner {
    type Error = ComposeRunnerError;

    fn run<'a>(
        &'a self,
        plan: &'a ExecutionPlan,
    ) -> testing_framework_core::scenario::BoxFuture<'a, Result<RunHandle, Self::Error>> {
        async move {
            let descriptors = plan.build_topology();
            validate_topology(&descriptors)?;

            let prometheus_port = desired_prometheus_port(plan);
            let environment = prepare_environment(&descriptors, prometheus_port).await?;
            orchestrate_stack(plan, descriptors, environment).await
        }
        .boxed()
    }
}

async fn orchestrate_stack(
    plan: &ExecutionPlan,
    descriptors: GeneratedTopology,
    mut environment: StackEnvironment,
) -> Result<RunHandle, ComposeRunnerError> {
    ensure_validators_ready(&descriptors, &mut environment).await?;
    ensure_remote_readiness(plan, &descriptors, &mut environment).await?;

    let validator_clients =
        build_validator_clients(&descriptors).map_err(ComposeRunnerError::Probe)?;
    let cleanup_guard = environment.into_cleanup();
    let metrics = resolve_metrics(plan);

    execute_plan(
        plan,
        plan.duration(),
        metrics,
        descriptors,
        validator_clients,
        cleanup_guard,
    )
    .await
}

struct WorkspaceState {
    workspace: ComposeWorkspace,
    root: PathBuf,
    cfgsync_path: PathBuf,
    use_kzg: bool,
}

fn validate_topology(descriptors: &GeneratedTopology) -> Result<(), ComposeRunnerError> {
    let validators = descriptors.validators().len();
    let executors = descriptors.executors().len();
    if validators != 1 || executors != 1 {
        return Err(ComposeRunnerError::UnsupportedTopology {
            validators,
            executors,
        });
    }
    Ok(())
}

fn desired_prometheus_port(plan: &ExecutionPlan) -> u16 {
    plan.metrics()
        .prometheus()
        .and_then(|endpoint| endpoint.port())
        .or_else(|| {
            let raw = env::var(PROMETHEUS_PORT_ENV).ok()?;
            raw.parse::<u16>().ok()
        })
        .unwrap_or(9090)
}

fn prepare_workspace() -> Result<WorkspaceState, String> {
    let workspace =
        ComposeWorkspace::create().map_err(|err| format!("creating workspace: {err}"))?;

    let root = workspace.root_path().to_path_buf();
    let cfgsync_path = workspace.testnet_dir().join("cfgsync.yaml");
    let use_kzg = workspace.root_path().join("kzgrs_test_params").exists();
    // cfgsync config will be updated later using descriptors
    Ok(WorkspaceState {
        workspace,
        root,
        cfgsync_path,
        use_kzg,
    })
}

fn allocate_cfgsync_port() -> Result<u16, anyhow::Error> {
    let listener =
        StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).context("allocating cfgsync port")?;

    let port = listener
        .local_addr()
        .context("reading cfgsync port")?
        .port();
    Ok(port)
}

fn write_compose(
    workflow: &WorkspaceState,
    descriptors: &GeneratedTopology,
    cfgsync_port: u16,
    prometheus_port: u16,
) -> anyhow::Result<PathBuf> {
    let compose_path = workflow.root.join("compose.generated.yml");
    write_compose_file(
        &compose_path,
        workflow.use_kzg,
        descriptors,
        cfgsync_port,
        prometheus_port,
    )?;
    Ok(compose_path)
}

fn collect_validator_ports(descriptors: &GeneratedTopology) -> Vec<u16> {
    descriptors
        .validators()
        .iter()
        .map(GeneratedNodeConfig::api_port)
        .collect()
}

async fn ensure_validators_ready(
    descriptors: &GeneratedTopology,
    environment: &mut StackEnvironment,
) -> Result<(), ComposeRunnerError> {
    let validator_ports = collect_validator_ports(descriptors);
    if let Err(err) = wait_for_validators(&validator_ports).await {
        teardown_environment(environment).await;
        return Err(ComposeRunnerError::Probe(err));
    }

    Ok(())
}

async fn ensure_remote_readiness(
    plan: &ExecutionPlan,
    descriptors: &GeneratedTopology,
    environment: &mut StackEnvironment,
) -> Result<(), ComposeRunnerError> {
    if !plan.readiness_enabled() {
        return Ok(());
    }

    let (validator_urls, executor_urls) = readiness_urls(descriptors)?;
    if let Err(err) = descriptors
        .wait_remote_readiness(&validator_urls, &executor_urls, None, None)
        .await
    {
        teardown_environment(environment).await;
        return Err(ComposeRunnerError::Probe(err.to_string()));
    }

    Ok(())
}

fn readiness_urls(
    descriptors: &GeneratedTopology,
) -> Result<(Vec<Url>, Vec<Url>), ComposeRunnerError> {
    let validator_urls = descriptors
        .validators()
        .iter()
        .map(validator_base_url)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ComposeRunnerError::Probe)?;
    let executor_urls = descriptors
        .executors()
        .iter()
        .map(executor_base_url)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ComposeRunnerError::Probe)?;

    Ok((validator_urls, executor_urls))
}

fn build_validator_clients(
    descriptors: &GeneratedTopology,
) -> Result<Vec<NodeClientHandle>, String> {
    descriptors
        .validators()
        .iter()
        .map(|descriptor| {
            let base_url = validator_base_url(descriptor)?;
            let client = HttpNodeClient::new(descriptor.clone(), base_url);
            Ok(NodeClientHandle::new(Arc::new(client)))
        })
        .collect()
}

async fn teardown_environment(environment: &mut StackEnvironment) {
    dump_compose_logs(
        environment.compose_path(),
        environment.project_name(),
        environment.root(),
    )
    .await;

    Box::new(environment.take_cleanup()).cleanup();
}

fn validator_base_url(descriptor: &GeneratedNodeConfig) -> Result<Url, String> {
    Url::parse(&format!("http://127.0.0.1:{}/", descriptor.api_port()))
        .map_err(|e| format!("invalid validator url: {e}"))
}

fn executor_base_url(descriptor: &GeneratedNodeConfig) -> Result<Url, String> {
    Url::parse(&format!("http://127.0.0.1:{}/", descriptor.api_port()))
        .map_err(|e| format!("invalid executor url: {e}"))
}

async fn execute_plan(
    plan: &ExecutionPlan,
    run_duration: Duration,
    metrics: Metrics,
    descriptors: GeneratedTopology,
    validator_clients: Vec<NodeClientHandle>,
    cleanup_guard: RunnerCleanup,
) -> Result<RunHandle, ComposeRunnerError> {
    configure_plan_metrics(plan, &metrics);

    let context = RunContext::new(
        descriptors,
        None,
        validator_clients,
        metrics.clone(),
        run_duration,
    );

    let mut cleanup_guard = Some(cleanup_guard);
    run_workloads(plan, &context, &mut cleanup_guard).await?;
    wait_for_run_duration(run_duration).await;
    evaluate_expectations(plan, &context, &mut cleanup_guard).await?;

    let cleanup_guard: Box<dyn CleanupGuard> =
        Box::new(cleanup_guard.expect("cleanup guard should be available"));
    Ok(RunHandle::new(context, Some(cleanup_guard)))
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
    cleanup_guard: &mut Option<RunnerCleanup>,
) -> Result<(), ComposeRunnerError> {
    for workload in plan.workloads() {
        if let Err(source) = workload.start(context).await {
            teardown_and_cleanup(cleanup_guard).await;
            return Err(ComposeRunnerError::WorkloadFailed {
                name: workload.name().to_owned(),
                source,
            });
        }
    }
    Ok(())
}

async fn evaluate_expectations(
    plan: &ExecutionPlan,
    context: &RunContext,
    cleanup_guard: &mut Option<RunnerCleanup>,
) -> Result<(), ComposeRunnerError> {
    for expectation in plan.expectations() {
        if let Err(source) = expectation.evaluate(context).await {
            teardown_and_cleanup(cleanup_guard).await;
            return Err(ComposeRunnerError::ExpectationFailed {
                name: expectation.name().to_owned(),
                source,
            });
        }
    }
    Ok(())
}

async fn teardown_and_cleanup(guard: &mut Option<RunnerCleanup>) {
    if let Some(cleanup_guard) = guard.take() {
        dump_compose_logs(
            &cleanup_guard.compose_file,
            &cleanup_guard.project_name,
            &cleanup_guard.root,
        )
        .await;
        Box::new(cleanup_guard).cleanup();
    }
}

async fn wait_for_run_duration(duration: Duration) {
    if !duration.is_zero() {
        tokio::time::sleep(duration).await;
    }
}

fn resolve_metrics(plan: &ExecutionPlan) -> Metrics {
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
}

async fn prepare_environment(
    descriptors: &GeneratedTopology,
    prometheus_port: u16,
) -> Result<StackEnvironment, ComposeRunnerError> {
    let workspace = prepare_workspace().map_err(ComposeRunnerError::Workspace)?;

    update_cfgsync_config(&workspace.cfgsync_path, descriptors, workspace.use_kzg)
        .map_err(|e| ComposeRunnerError::Config(e.to_string()))?;

    let cfgsync_port =
        allocate_cfgsync_port().map_err(|e| ComposeRunnerError::Config(e.to_string()))?;

    let cfgsync_handle = start_cfgsync_server(&workspace.cfgsync_path, cfgsync_port)
        .await
        .map_err(|e| ComposeRunnerError::Config(e.to_string()))?;

    let compose_path = write_compose(&workspace, descriptors, cfgsync_port, prometheus_port)
        .map_err(|err| ComposeRunnerError::Config(err.to_string()))?;

    let project_name = format!("nomos-compose-{}", Uuid::new_v4());

    if let Err(err) = compose_up(&compose_path, &project_name, &workspace.root).await {
        let mut handle = cfgsync_handle;
        handle.shutdown();
        return Err(ComposeRunnerError::Compose(err));
    }

    Ok(StackEnvironment::from_workspace(
        workspace,
        compose_path,
        project_name,
        Some(cfgsync_handle),
    ))
}
