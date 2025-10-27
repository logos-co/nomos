use std::{
    env,
    path::{Path, PathBuf},
    process,
};

use anyhow::Context as _;
use serde::Serialize;
use tera::Context as TeraContext;
use testing_framework_core::topology::{GeneratedNodeConfig, GeneratedTopology};
use tokio::process::Command;

pub async fn compose_up(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
) -> Result<(), String> {
    let mut up_cmd = Command::new("docker");
    up_cmd
        .arg("compose")
        .arg("-f")
        .arg(compose_path)
        .arg("-p")
        .arg(project_name)
        .arg("up")
        .arg("-d")
        .current_dir(root);

    let status = up_cmd
        .status()
        .await
        .map_err(|e| format!("failed to spawn docker compose up: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("docker compose up exited with status {status}"))
    }
}

pub fn compose_down(compose_path: &Path, project_name: &str, root: &Path) -> Result<(), String> {
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
        .map_err(|e| format!("failed to spawn docker compose down: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("docker compose down exited with status {status}"))
    }
}

pub fn write_compose_file(
    compose_path: &Path,
    use_kzg_mount: bool,
    topology: &GeneratedTopology,
    cfgsync_port: u16,
    prometheus_port: u16,
) -> anyhow::Result<()> {
    let repo_root = repository_root()?;
    let template_path =
        repo_root.join("testing-framework/runners/compose/assets/docker-compose.yml.tera");
    let template = std::fs::read_to_string(&template_path)
        .with_context(|| format!("reading compose template at {}", template_path.display()))?;

    let image =
        env::var("NOMOS_TESTNET_IMAGE").unwrap_or_else(|_| String::from("nomos-testnet:local"));
    let platform = (image == "ghcr.io/logos-co/nomos:testnet").then(|| "linux/amd64".to_owned());

    let validators = topology
        .validators()
        .iter()
        .enumerate()
        .map(|(index, validator)| {
            NodeTemplate::from_validator(
                index,
                validator,
                &image,
                platform.as_deref(),
                use_kzg_mount,
                cfgsync_port,
            )
        })
        .collect::<Vec<_>>();

    let executors = topology
        .executors()
        .iter()
        .enumerate()
        .map(|(index, executor)| {
            NodeTemplate::from_executor(
                index,
                executor,
                &image,
                platform.as_deref(),
                use_kzg_mount,
                cfgsync_port,
            )
        })
        .collect::<Vec<_>>();

    let context = ComposeTemplate {
        prometheus: PrometheusTemplate {
            host_port: format!("127.0.0.1:{prometheus_port}:9090"),
        },
        validators,
        executors,
    };

    let rendered = tera::Tera::one_off(&template, &TeraContext::from_serialize(&context)?, false)
        .context("rendering compose template")?;

    std::fs::write(compose_path, rendered).context("writing compose file")?;
    Ok(())
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

fn repository_root() -> anyhow::Result<PathBuf> {
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

#[derive(Serialize)]
struct ComposeTemplate {
    prometheus: PrometheusTemplate,
    validators: Vec<NodeTemplate>,
    executors: Vec<NodeTemplate>,
}

#[derive(Serialize)]
struct PrometheusTemplate {
    host_port: String,
}

#[derive(Serialize)]
struct NodeTemplate {
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

#[derive(Serialize)]
struct EnvEntry {
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
}

impl NodeTemplate {
    fn from_validator(
        index: usize,
        validator: &GeneratedNodeConfig,
        image: &str,
        platform: Option<&str>,
        use_kzg_mount: bool,
        cfgsync_port: u16,
    ) -> Self {
        let mut environment = Self::base_environment(cfgsync_port);
        environment.extend([
            EnvEntry::new(
                "CFG_NETWORK_PORT",
                validator
                    .general
                    .network_config
                    .swarm_config
                    .port
                    .to_string(),
            ),
            EnvEntry::new("CFG_DA_PORT", validator.da_port.to_string()),
            EnvEntry::new("CFG_BLEND_PORT", validator.blend_port.to_string()),
            EnvEntry::new("CFG_API_PORT", "18080"),
            EnvEntry::new(
                "CFG_TESTING_HTTP_PORT",
                validator.testing_http_port().to_string(),
            ),
        ]);

        let ports = vec![
            format!("{}:18080", validator.api_port()),
            format!(
                "{}:{}",
                validator.testing_http_port(),
                validator.testing_http_port()
            ),
        ];

        Self {
            name: format!("validator-{index}"),
            image: image.to_owned(),
            entrypoint: "/etc/nomos/scripts/run_nomos_node.sh".into(),
            volumes: Self::base_volumes(use_kzg_mount),
            extra_hosts: vec!["host.docker.internal:host-gateway".into()],
            ports,
            environment,
            platform: platform.map(ToString::to_string),
        }
    }

    fn from_executor(
        index: usize,
        executor: &GeneratedNodeConfig,
        image: &str,
        platform: Option<&str>,
        use_kzg_mount: bool,
        cfgsync_port: u16,
    ) -> Self {
        let mut environment = Self::base_environment(cfgsync_port);
        environment.extend([
            EnvEntry::new(
                "CFG_NETWORK_PORT",
                executor
                    .general
                    .network_config
                    .swarm_config
                    .port
                    .to_string(),
            ),
            EnvEntry::new("CFG_DA_PORT", executor.da_port.to_string()),
            EnvEntry::new("CFG_BLEND_PORT", executor.blend_port.to_string()),
            EnvEntry::new(
                "CFG_API_PORT",
                executor.general.api_config.address.port().to_string(),
            ),
            EnvEntry::new(
                "CFG_TESTING_HTTP_PORT",
                executor.testing_http_port().to_string(),
            ),
        ]);

        let ports = vec![
            format!(
                "{}:{}",
                executor.api_port(),
                executor.general.api_config.address.port()
            ),
            format!(
                "{}:{}",
                executor.testing_http_port(),
                executor.testing_http_port()
            ),
        ];

        Self {
            name: format!("executor-{index}"),
            image: image.to_owned(),
            entrypoint: "/etc/nomos/scripts/run_nomos_executor.sh".into(),
            volumes: Self::base_volumes(use_kzg_mount),
            extra_hosts: vec!["host.docker.internal:host-gateway".into()],
            ports,
            environment,
            platform: platform.map(ToString::to_string),
        }
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
}
