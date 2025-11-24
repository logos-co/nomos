use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process,
    time::Duration,
};

use anyhow::Context as _;
use serde::Serialize;
use tera::Context as TeraContext;
use testing_framework_core::{
    adjust_timeout,
    topology::{GeneratedNodeConfig, GeneratedTopology},
};
use tokio::{process::Command, time::timeout};

const COMPOSE_UP_TIMEOUT: Duration = Duration::from_secs(120);
const TEMPLATE_RELATIVE_PATH: &str =
    "testing-framework/runners/compose/assets/docker-compose.yml.tera";

#[derive(Debug, thiserror::Error)]
pub enum ComposeCommandError {
    #[error("{command} exited with status {status}")]
    Failed {
        command: String,
        status: process::ExitStatus,
    },
    #[error("failed to spawn {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("{command} timed out after {timeout:?}")]
    Timeout { command: String, timeout: Duration },
}

pub async fn compose_up(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
) -> Result<(), ComposeCommandError> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_path)
        .arg("-p")
        .arg(project_name)
        .arg("up")
        .arg("-d")
        .current_dir(root);

    run_compose_command(cmd, adjust_timeout(COMPOSE_UP_TIMEOUT), "docker compose up").await
}

pub fn compose_down(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
) -> Result<(), ComposeCommandError> {
    let description = "docker compose down".to_owned();
    let status = process::Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose_path)
        .arg("-p")
        .arg(project_name)
        .arg("down")
        .arg("--volumes")
        .current_dir(root)
        .status()
        .map_err(|source| ComposeCommandError::Spawn {
            command: description.clone(),
            source,
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(ComposeCommandError::Failed {
            command: description,
            status,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("failed to resolve repository root for compose template: {source}")]
    RepositoryRoot {
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to read compose template at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to serialise compose descriptor for templating: {source}")]
    Serialize {
        #[source]
        source: tera::Error,
    },
    #[error("failed to render compose template at {path}: {source}")]
    Render {
        path: PathBuf,
        #[source]
        source: tera::Error,
    },
    #[error("failed to write compose file at {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum DescriptorBuildError {
    #[error("cfgsync port is not configured for compose descriptor")]
    MissingCfgsyncPort,
    #[error("prometheus port is not configured for compose descriptor")]
    MissingPrometheusPort,
}

#[derive(Clone, Debug, Serialize)]
pub struct ComposeDescriptor {
    prometheus: PrometheusTemplate,
    validators: Vec<NodeDescriptor>,
    executors: Vec<NodeDescriptor>,
}

impl ComposeDescriptor {
    #[must_use]
    pub const fn builder(topology: &GeneratedTopology) -> ComposeDescriptorBuilder<'_> {
        ComposeDescriptorBuilder::new(topology)
    }

    #[cfg(test)]
    fn validators(&self) -> &[NodeDescriptor] {
        &self.validators
    }

    #[cfg(test)]
    fn executors(&self) -> &[NodeDescriptor] {
        &self.executors
    }
}

pub struct ComposeDescriptorBuilder<'a> {
    topology: &'a GeneratedTopology,
    use_kzg_mount: bool,
    cfgsync_port: Option<u16>,
    prometheus_port: Option<u16>,
}

impl<'a> ComposeDescriptorBuilder<'a> {
    const fn new(topology: &'a GeneratedTopology) -> Self {
        Self {
            topology,
            use_kzg_mount: false,
            cfgsync_port: None,
            prometheus_port: None,
        }
    }

    #[must_use]
    pub const fn with_kzg_mount(mut self, enabled: bool) -> Self {
        self.use_kzg_mount = enabled;
        self
    }

    #[must_use]
    pub const fn with_cfgsync_port(mut self, port: u16) -> Self {
        self.cfgsync_port = Some(port);
        self
    }

    #[must_use]
    pub const fn with_prometheus_port(mut self, port: u16) -> Self {
        self.prometheus_port = Some(port);
        self
    }

    pub fn build(self) -> Result<ComposeDescriptor, DescriptorBuildError> {
        let cfgsync_port = self
            .cfgsync_port
            .ok_or(DescriptorBuildError::MissingCfgsyncPort)?;
        let prometheus_host_port = self
            .prometheus_port
            .ok_or(DescriptorBuildError::MissingPrometheusPort)?;

        let (default_image, default_platform) = resolve_image();
        let image = default_image;
        let platform = default_platform;

        let validators = build_nodes(
            self.topology.validators(),
            ComposeNodeKind::Validator,
            &image,
            platform.as_deref(),
            self.use_kzg_mount,
            cfgsync_port,
        );

        let executors = build_nodes(
            self.topology.executors(),
            ComposeNodeKind::Executor,
            &image,
            platform.as_deref(),
            self.use_kzg_mount,
            cfgsync_port,
        );

        Ok(ComposeDescriptor {
            prometheus: PrometheusTemplate::new(prometheus_host_port),
            validators,
            executors,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PrometheusTemplate {
    host_port: String,
}

impl PrometheusTemplate {
    fn new(port: u16) -> Self {
        Self {
            host_port: format!("127.0.0.1:{port}:9090"),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct EnvEntry {
    key: String,
    value: String,
}

impl EnvEntry {
    fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    #[cfg(test)]
    fn key(&self) -> &str {
        &self.key
    }

    #[cfg(test)]
    fn value(&self) -> &str {
        &self.value
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct NodeDescriptor {
    name: String,
    image: String,
    entrypoint: String,
    volumes: Vec<String>,
    extra_hosts: Vec<String>,
    ports: Vec<String>,
    environment: Vec<EnvEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NodeHostPorts {
    pub api: u16,
    pub testing: u16,
}

#[derive(Clone, Debug)]
pub struct HostPortMapping {
    pub validators: Vec<NodeHostPorts>,
    pub executors: Vec<NodeHostPorts>,
}

impl HostPortMapping {
    pub fn validator_api_ports(&self) -> Vec<u16> {
        self.validators.iter().map(|ports| ports.api).collect()
    }

    pub fn executor_api_ports(&self) -> Vec<u16> {
        self.executors.iter().map(|ports| ports.api).collect()
    }
}

impl NodeDescriptor {
    fn from_node(
        kind: ComposeNodeKind,
        index: usize,
        node: &GeneratedNodeConfig,
        image: &str,
        platform: Option<&str>,
        use_kzg_mount: bool,
        cfgsync_port: u16,
    ) -> Self {
        let mut environment = base_environment(cfgsync_port);
        let identifier = kind.instance_name(index);
        environment.extend([
            EnvEntry::new(
                "CFG_NETWORK_PORT",
                node.general.network_config.swarm_config.port.to_string(),
            ),
            EnvEntry::new("CFG_DA_PORT", node.da_port.to_string()),
            EnvEntry::new("CFG_BLEND_PORT", node.blend_port.to_string()),
            EnvEntry::new(
                "CFG_API_PORT",
                node.general.api_config.address.port().to_string(),
            ),
            EnvEntry::new(
                "CFG_TESTING_HTTP_PORT",
                node.general
                    .api_config
                    .testing_http_address
                    .port()
                    .to_string(),
            ),
            EnvEntry::new("CFG_HOST_IDENTIFIER", identifier),
        ]);

        let ports = vec![
            node.general.api_config.address.port().to_string(),
            node.general
                .api_config
                .testing_http_address
                .port()
                .to_string(),
        ];

        Self {
            name: kind.instance_name(index),
            image: image.to_owned(),
            entrypoint: kind.entrypoint().to_owned(),
            volumes: base_volumes(use_kzg_mount),
            extra_hosts: default_extra_hosts(),
            ports,
            environment,
            platform: platform.map(ToOwned::to_owned),
        }
    }

    #[cfg(test)]
    fn ports(&self) -> &[String] {
        &self.ports
    }

    #[cfg(test)]
    fn environment(&self) -> &[EnvEntry] {
        &self.environment
    }
}

pub fn write_compose_file(
    descriptor: &ComposeDescriptor,
    compose_path: &Path,
) -> Result<(), TemplateError> {
    TemplateSource::load()?.write(descriptor, compose_path)
}

pub async fn dump_compose_logs(compose_file: &Path, project: &str, root: &Path) {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("-p")
        .arg(project)
        .arg("logs")
        .arg("--no-color")
        .current_dir(root);

    match cmd.output().await {
        Ok(output) => {
            if !output.stdout.is_empty() {
                eprintln!(
                    "[compose-runner] docker compose logs:\n{}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            if !output.stderr.is_empty() {
                eprintln!(
                    "[compose-runner] docker compose errors:\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
        Err(err) => {
            eprintln!("[compose-runner] failed to collect docker compose logs: {err}");
        }
    }
}

struct TemplateSource {
    path: PathBuf,
    contents: String,
}

impl TemplateSource {
    fn load() -> Result<Self, TemplateError> {
        let repo_root =
            repository_root().map_err(|source| TemplateError::RepositoryRoot { source })?;
        let path = repo_root.join(TEMPLATE_RELATIVE_PATH);
        let contents = fs::read_to_string(&path).map_err(|source| TemplateError::Read {
            path: path.clone(),
            source,
        })?;

        Ok(Self { path, contents })
    }

    fn render(&self, descriptor: &ComposeDescriptor) -> Result<String, TemplateError> {
        let context = TeraContext::from_serialize(descriptor)
            .map_err(|source| TemplateError::Serialize { source })?;

        tera::Tera::one_off(&self.contents, &context, false).map_err(|source| {
            TemplateError::Render {
                path: self.path.clone(),
                source,
            }
        })
    }

    fn write(&self, descriptor: &ComposeDescriptor, output: &Path) -> Result<(), TemplateError> {
        let rendered = self.render(descriptor)?;
        fs::write(output, rendered).map_err(|source| TemplateError::Write {
            path: output.to_path_buf(),
            source,
        })
    }
}

pub fn repository_root() -> anyhow::Result<PathBuf> {
    env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .map(PathBuf::from)
                .context("resolving repository root from manifest dir")
        })
}

#[derive(Clone, Copy)]
enum ComposeNodeKind {
    Validator,
    Executor,
}

impl ComposeNodeKind {
    fn instance_name(self, index: usize) -> String {
        match self {
            Self::Validator => format!("validator-{index}"),
            Self::Executor => format!("executor-{index}"),
        }
    }

    const fn entrypoint(self) -> &'static str {
        match self {
            Self::Validator => "/etc/nomos/scripts/run_nomos_node.sh",
            Self::Executor => "/etc/nomos/scripts/run_nomos_executor.sh",
        }
    }
}

fn build_nodes(
    nodes: &[GeneratedNodeConfig],
    kind: ComposeNodeKind,
    image: &str,
    platform: Option<&str>,
    use_kzg_mount: bool,
    cfgsync_port: u16,
) -> Vec<NodeDescriptor> {
    nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            NodeDescriptor::from_node(
                kind,
                index,
                node,
                image,
                platform,
                use_kzg_mount,
                cfgsync_port,
            )
        })
        .collect()
}

fn base_environment(cfgsync_port: u16) -> Vec<EnvEntry> {
    vec![
        EnvEntry::new("POL_PROOF_DEV_MODE", "true"),
        EnvEntry::new(
            "CFG_SERVER_ADDR",
            format!("http://host.docker.internal:{cfgsync_port}"),
        ),
        EnvEntry::new("OTEL_METRIC_EXPORT_INTERVAL", "5000"),
    ]
}

fn base_volumes(use_kzg_mount: bool) -> Vec<String> {
    let mut volumes = vec!["./testnet:/etc/nomos".into()];
    if use_kzg_mount {
        volumes.push("./kzgrs_test_params:/kzgrs_test_params:z".into());
    }
    volumes
}

fn default_extra_hosts() -> Vec<String> {
    host_gateway_entry().into_iter().collect()
}

pub fn resolve_image() -> (String, Option<String>) {
    let image =
        env::var("NOMOS_TESTNET_IMAGE").unwrap_or_else(|_| String::from("nomos-testnet:local"));
    let platform = (image == "ghcr.io/logos-co/nomos:testnet").then(|| "linux/amd64".to_owned());
    (image, platform)
}

fn host_gateway_entry() -> Option<String> {
    if let Ok(value) = env::var("COMPOSE_RUNNER_HOST_GATEWAY") {
        if value.eq_ignore_ascii_case("disable") || value.is_empty() {
            return None;
        }
        return Some(value);
    }

    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return Some("host.docker.internal:host-gateway".into());
    }

    env::var("DOCKER_HOST_GATEWAY")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|gateway| format!("host.docker.internal:{gateway}"))
}

async fn run_compose_command(
    mut command: Command,
    timeout_duration: Duration,
    description: &str,
) -> Result<(), ComposeCommandError> {
    match timeout(timeout_duration, command.status()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(ComposeCommandError::Failed {
            command: description.to_owned(),
            status,
        }),
        Ok(Err(err)) => Err(ComposeCommandError::Spawn {
            command: description.to_owned(),
            source: err,
        }),
        Err(_) => Err(ComposeCommandError::Timeout {
            command: description.to_owned(),
            timeout: timeout_duration,
        }),
    }
}

#[cfg(test)]
mod tests {
    use testing_framework_core::topology::{TopologyBuilder, TopologyConfig};

    use super::*;

    #[test]
    fn descriptor_matches_topology_counts() {
        let topology = TopologyBuilder::new(TopologyConfig::with_node_numbers(2, 1)).build();
        let descriptor = ComposeDescriptor::builder(&topology)
            .with_cfgsync_port(4400)
            .with_prometheus_port(9090)
            .build()
            .expect("descriptor");

        assert_eq!(descriptor.validators().len(), topology.validators().len());
        assert_eq!(descriptor.executors().len(), topology.executors().len());
    }

    #[test]
    fn descriptor_includes_expected_env_and_ports() {
        let topology = TopologyBuilder::new(TopologyConfig::with_node_numbers(1, 1)).build();
        let cfgsync_port = 4555;
        let descriptor = ComposeDescriptor::builder(&topology)
            .with_cfgsync_port(cfgsync_port)
            .with_prometheus_port(9090)
            .build()
            .expect("descriptor");

        let validator = &descriptor.validators()[0];
        assert!(
            validator
                .environment()
                .iter()
                .any(|entry| entry.key() == "CFG_SERVER_ADDR"
                    && entry.value() == format!("http://host.docker.internal:{cfgsync_port}"))
        );

        let api_container = topology.validators()[0].general.api_config.address.port();
        assert!(validator.ports().contains(&api_container.to_string()));
    }
}
