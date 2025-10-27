use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result as AnyResult};
use serde::Serialize;
use tempfile::TempDir;
use testing_framework_core::{
    scenario::cfgsync::{apply_topology_overrides, load_cfgsync_template, render_cfgsync_yaml},
    topology::GeneratedTopology,
};

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

pub fn prepare_assets(topology: &GeneratedTopology) -> AnyResult<RunnerAssets> {
    let root = workspace_root()?;

    let cfgsync_template_path = root.join("testnet/cfgsync.yaml");
    let mut cfgsync = load_cfgsync_template(&cfgsync_template_path)?;
    apply_topology_overrides(&mut cfgsync, topology, true);
    let cfgsync_yaml = render_cfgsync_yaml(&cfgsync)?;

    let tempdir = tempfile::Builder::new()
        .prefix("nomos-helm-")
        .tempdir()
        .context("creating temporary directory for rendered assets")?;
    let cfgsync_file = tempdir.path().join("cfgsync.yaml");
    fs::write(&cfgsync_file, cfgsync_yaml).context("writing rendered cfgsync.yaml")?;

    let run_cfgsync_script = root.join("testnet/scripts/run_cfgsync.sh");
    if !run_cfgsync_script.exists() {
        anyhow::bail!(
            "run_cfgsync.sh not found at {}; build assets with `cargo build -p testnet` or ensure scripts are present",
            run_cfgsync_script.display()
        );
    }
    let run_nomos_node_script = root.join("testnet/scripts/run_nomos_node.sh");
    if !run_nomos_node_script.exists() {
        anyhow::bail!(
            "run_nomos_node.sh not found at {}",
            run_nomos_node_script.display()
        );
    }
    let run_nomos_executor_script = root.join("testnet/scripts/run_nomos_executor.sh");
    if !run_nomos_executor_script.exists() {
        anyhow::bail!(
            "run_nomos_executor.sh not found at {}",
            run_nomos_executor_script.display()
        );
    }

    let kzg_path = root.join("tests/kzgrs/kzgrs_test_params");
    if !kzg_path.exists() {
        anyhow::bail!(
            "expected KZG params at {}; build them with `make kzgrs_test_params`",
            kzg_path.display()
        );
    }

    let chart_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("helm/nomos-runner");
    if !chart_path.exists() {
        anyhow::bail!(
            "helm chart not found at {}; ensure the repository is up-to-date",
            chart_path.display()
        );
    }

    let image =
        env::var("NOMOS_TESTNET_IMAGE").unwrap_or_else(|_| String::from("nomos-testnet:local"));

    let values = build_values(topology);
    let values_yaml =
        serde_yaml::to_string(&values).context("serializing rendered Helm values overrides")?;
    let values_file = tempdir.path().join("values.yaml");
    fs::write(&values_file, values_yaml).context("writing rendered Helm values overrides")?;

    Ok(RunnerAssets {
        image,
        kzg_path,
        chart_path,
        cfgsync_file,
        run_cfgsync_script,
        run_nomos_node_script,
        run_nomos_executor_script,
        values_file,
        _tempdir: tempdir,
    })
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
