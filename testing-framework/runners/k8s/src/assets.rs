use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result as AnyResult};
use serde::Serialize;
use tempfile::TempDir;
use testing_framework_core::{
    scenario::cfgsync::{apply_topology_overrides, load_cfgsync_template, render_cfgsync_yaml},
    topology::GeneratedTopology,
};
use thiserror::Error;

pub struct RunnerAssets {
    pub image: String,
    pub kzg_path: PathBuf,
    pub chart_path: PathBuf,
    pub cfgsync_file: PathBuf,
    pub run_cfgsync_script: PathBuf,
    pub run_nomos_node_script: PathBuf,
    pub run_nomos_executor_script: PathBuf,
    pub values_file: PathBuf,
    _tempdir: TempDir,
}

pub const CFGSYNC_PORT: u16 = 4400;

#[derive(Debug, Error)]
pub enum AssetsError {
    #[error("failed to locate workspace root: {source}")]
    WorkspaceRoot {
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to render cfgsync configuration: {source}")]
    Cfgsync {
        #[source]
        source: anyhow::Error,
    },
    #[error("missing required script at {path}")]
    MissingScript { path: PathBuf },
    #[error("missing KZG parameters at {path}; build them with `make kzgrs_test_params`")]
    MissingKzg { path: PathBuf },
    #[error("missing Helm chart at {path}; ensure the repository is up-to-date")]
    MissingChart { path: PathBuf },
    #[error("failed to create temporary directory for rendered assets: {source}")]
    TempDir {
        #[source]
        source: io::Error,
    },
    #[error("failed to write asset at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to render Helm values: {source}")]
    Values {
        #[source]
        source: serde_yaml::Error,
    },
}

pub fn prepare_assets(topology: &GeneratedTopology) -> Result<RunnerAssets, AssetsError> {
    let root = workspace_root().map_err(|source| AssetsError::WorkspaceRoot { source })?;
    let cfgsync_yaml = render_cfgsync_config(&root, topology)?;

    let tempdir = tempfile::Builder::new()
        .prefix("nomos-helm-")
        .tempdir()
        .map_err(|source| AssetsError::TempDir { source })?;

    let cfgsync_file = write_temp_file(tempdir.path(), "cfgsync.yaml", cfgsync_yaml)?;
    let scripts = validate_scripts(&root)?;
    let kzg_path = validate_kzg_params(&root)?;
    let chart_path = helm_chart_path()?;
    let values_yaml = render_values_yaml(topology)?;
    let values_file = write_temp_file(tempdir.path(), "values.yaml", values_yaml)?;
    let image =
        env::var("NOMOS_TESTNET_IMAGE").unwrap_or_else(|_| String::from("nomos-testnet:local"));

    Ok(RunnerAssets {
        image,
        kzg_path,
        chart_path,
        cfgsync_file,
        run_cfgsync_script: scripts.run_cfgsync,
        run_nomos_node_script: scripts.run_node,
        run_nomos_executor_script: scripts.run_executor,
        values_file,
        _tempdir: tempdir,
    })
}

const CFGSYNC_K8S_TIMEOUT_SECS: u64 = 300;

fn render_cfgsync_config(root: &Path, topology: &GeneratedTopology) -> Result<String, AssetsError> {
    let cfgsync_template_path = root.join("testnet/cfgsync.yaml");
    let mut cfg = load_cfgsync_template(&cfgsync_template_path)
        .map_err(|source| AssetsError::Cfgsync { source })?;
    apply_topology_overrides(&mut cfg, topology, true);
    cfg.timeout = cfg.timeout.max(CFGSYNC_K8S_TIMEOUT_SECS);
    render_cfgsync_yaml(&cfg).map_err(|source| AssetsError::Cfgsync { source })
}

struct ScriptPaths {
    run_cfgsync: PathBuf,
    run_node: PathBuf,
    run_executor: PathBuf,
}

fn validate_scripts(root: &Path) -> Result<ScriptPaths, AssetsError> {
    let scripts_dir = root.join("testnet/scripts");
    let run_cfgsync = scripts_dir.join("run_cfgsync.sh");
    let run_node = scripts_dir.join("run_nomos_node.sh");
    let run_executor = scripts_dir.join("run_nomos_executor.sh");

    for path in [&run_cfgsync, &run_node, &run_executor] {
        if !path.exists() {
            return Err(AssetsError::MissingScript { path: path.clone() });
        }
    }

    Ok(ScriptPaths {
        run_cfgsync,
        run_node,
        run_executor,
    })
}

fn validate_kzg_params(root: &Path) -> Result<PathBuf, AssetsError> {
    let path = root.join("tests/kzgrs/kzgrs_test_params");
    if path.exists() {
        Ok(path)
    } else {
        Err(AssetsError::MissingKzg { path })
    }
}

fn helm_chart_path() -> Result<PathBuf, AssetsError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("helm/nomos-runner");
    if path.exists() {
        Ok(path)
    } else {
        Err(AssetsError::MissingChart { path })
    }
}

fn render_values_yaml(topology: &GeneratedTopology) -> Result<String, AssetsError> {
    let values = build_values(topology);
    serde_yaml::to_string(&values).map_err(|source| AssetsError::Values { source })
}

fn write_temp_file(
    dir: &Path,
    name: &str,
    contents: impl AsRef<[u8]>,
) -> Result<PathBuf, AssetsError> {
    let path = dir.join(name);
    fs::write(&path, contents).map_err(|source| AssetsError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn workspace_root() -> AnyResult<PathBuf> {
    if let Ok(var) = env::var("CARGO_WORKSPACE_DIR") {
        return Ok(PathBuf::from(var));
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .context("resolving workspace root from manifest dir")
}

#[derive(Serialize)]
struct HelmValues {
    validators: NodeGroup,
    executors: NodeGroup,
}

#[derive(Serialize)]
struct NodeGroup {
    count: usize,
    nodes: Vec<NodeValues>,
}

#[derive(Serialize)]
struct NodeValues {
    #[serde(rename = "apiPort")]
    api_port: u16,
    #[serde(rename = "testingHttpPort")]
    testing_http_port: u16,
    env: BTreeMap<String, String>,
}

fn build_values(topology: &GeneratedTopology) -> HelmValues {
    let validators = topology
        .validators()
        .iter()
        .map(|validator| {
            let mut env = BTreeMap::new();
            env.insert(
                "CFG_NETWORK_PORT".into(),
                validator.network_port().to_string(),
            );
            env.insert("CFG_DA_PORT".into(), validator.da_port.to_string());
            env.insert("CFG_BLEND_PORT".into(), validator.blend_port.to_string());
            env.insert(
                "CFG_API_PORT".into(),
                validator.general.api_config.address.port().to_string(),
            );
            env.insert(
                "CFG_TESTING_HTTP_PORT".into(),
                validator
                    .general
                    .api_config
                    .testing_http_address
                    .port()
                    .to_string(),
            );

            NodeValues {
                api_port: validator.general.api_config.address.port(),
                testing_http_port: validator.general.api_config.testing_http_address.port(),
                env,
            }
        })
        .collect();

    let executors = topology
        .executors()
        .iter()
        .map(|executor| {
            let mut env = BTreeMap::new();
            env.insert(
                "CFG_NETWORK_PORT".into(),
                executor.network_port().to_string(),
            );
            env.insert("CFG_DA_PORT".into(), executor.da_port.to_string());
            env.insert("CFG_BLEND_PORT".into(), executor.blend_port.to_string());
            env.insert(
                "CFG_API_PORT".into(),
                executor.general.api_config.address.port().to_string(),
            );
            env.insert(
                "CFG_TESTING_HTTP_PORT".into(),
                executor
                    .general
                    .api_config
                    .testing_http_address
                    .port()
                    .to_string(),
            );

            NodeValues {
                api_port: executor.general.api_config.address.port(),
                testing_http_port: executor.general.api_config.testing_http_address.port(),
                env,
            }
        })
        .collect();

    HelmValues {
        validators: NodeGroup {
            count: topology.validators().len(),
            nodes: validators,
        },
        executors: NodeGroup {
            count: topology.executors().len(),
            nodes: executors,
        },
    }
}
