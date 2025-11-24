use std::{io, process::Stdio};

use thiserror::Error;
use tokio::process::Command;

use crate::assets::{CFGSYNC_PORT, RunnerAssets, workspace_root};

#[derive(Debug, Error)]
pub enum HelmError {
    #[error("failed to spawn {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("{command} exited with status {status:?}\nstderr:\n{stderr}\nstdout:\n{stdout}")]
    Failed {
        command: String,
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
}

pub async fn install_release(
    assets: &RunnerAssets,
    release: &str,
    namespace: &str,
    validators: usize,
    executors: usize,
) -> Result<(), HelmError> {
    let host_path_type = if assets.kzg_path.is_dir() {
        "Directory"
    } else {
        "File"
    };

    let mut cmd = Command::new("helm");
    cmd.arg("install")
        .arg(release)
        .arg(&assets.chart_path)
        .arg("--namespace")
        .arg(namespace)
        .arg("--create-namespace")
        .arg("--wait")
        .arg("--timeout")
        .arg("5m")
        .arg("--set")
        .arg(format!("image={}", assets.image))
        .arg("--set")
        .arg(format!("validators.count={validators}"))
        .arg("--set")
        .arg(format!("executors.count={executors}"))
        .arg("--set")
        .arg(format!("cfgsync.port={CFGSYNC_PORT}"))
        .arg("--set")
        .arg(format!("kzg.hostPath={}", assets.kzg_path.display()))
        .arg("--set")
        .arg(format!("kzg.hostPathType={host_path_type}"))
        .arg("-f")
        .arg(&assets.values_file)
        .arg("--set-file")
        .arg(format!("cfgsync.config={}", assets.cfgsync_file.display()))
        .arg("--set-file")
        .arg(format!(
            "scripts.runCfgsyncSh={}",
            assets.run_cfgsync_script.display()
        ))
        .arg("--set-file")
        .arg(format!(
            "scripts.runNomosNodeSh={}",
            assets.run_nomos_node_script.display()
        ))
        .arg("--set-file")
        .arg(format!(
            "scripts.runNomosExecutorSh={}",
            assets.run_nomos_executor_script.display()
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Ok(root) = workspace_root() {
        cmd.current_dir(root);
    }

    let command = format!("helm install {release}");
    let output = run_helm_command(cmd, &command).await?;

    if std::env::var("K8S_RUNNER_DEBUG").is_ok() {
        println!(
            "[k8s-runner] {command} stdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        println!(
            "[k8s-runner] {command} stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

pub async fn uninstall_release(release: &str, namespace: &str) -> Result<(), HelmError> {
    let mut cmd = Command::new("helm");
    cmd.arg("uninstall")
        .arg(release)
        .arg("--namespace")
        .arg(namespace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    println!("[k8s-runner] issuing `helm uninstall {release}` in namespace `{namespace}`");

    run_helm_command(cmd, &format!("helm uninstall {release}")).await?;
    println!(
        "[k8s-runner] helm uninstall {release} completed successfully (namespace `{namespace}`)"
    );
    Ok(())
}

async fn run_helm_command(
    mut cmd: Command,
    command: &str,
) -> Result<std::process::Output, HelmError> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = cmd.output().await.map_err(|source| HelmError::Spawn {
        command: command.to_owned(),
        source,
    })?;

    if output.status.success() {
        Ok(output)
    } else {
        Err(HelmError::Failed {
            command: command.to_owned(),
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}
