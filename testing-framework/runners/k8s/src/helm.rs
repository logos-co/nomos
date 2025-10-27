use std::process::Stdio;

use tokio::process::Command;

use crate::assets::{CFGSYNC_PORT, RunnerAssets, workspace_root};

pub async fn install_release(
    assets: &RunnerAssets,
    release: &str,
    namespace: &str,
    validators: usize,
    executors: usize,
) -> Result<(), String> {
    let chart_path = assets
        .chart_path
        .to_str()
        .ok_or_else(|| "invalid chart path".to_owned())?;
    let cfgsync_file = assets
        .cfgsync_file
        .to_str()
        .ok_or_else(|| "invalid cfgsync file path".to_owned())?;
    let run_cfgsync_script = assets
        .run_cfgsync_script
        .to_str()
        .ok_or_else(|| "invalid run_cfgsync.sh path".to_owned())?;
    let run_nomos_node_script = assets
        .run_nomos_node_script
        .to_str()
        .ok_or_else(|| "invalid run_nomos_node.sh path".to_owned())?;
    let run_nomos_executor_script = assets
        .run_nomos_executor_script
        .to_str()
        .ok_or_else(|| "invalid run_nomos_executor.sh path".to_owned())?;
    let kzg_path = assets
        .kzg_path
        .to_str()
        .ok_or_else(|| "invalid kzg path".to_owned())?;
    let values_file = assets
        .values_file
        .to_str()
        .ok_or_else(|| "invalid Helm values file path".to_owned())?;
    let host_path_type = if assets.kzg_path.is_dir() {
        "Directory"
    } else {
        "File"
    };

    let mut cmd = Command::new("helm");
    cmd.arg("install")
        .arg(release)
        .arg(chart_path)
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
        .arg(format!("kzg.hostPath={kzg_path}"))
        .arg("--set")
        .arg(format!("kzg.hostPathType={host_path_type}"))
        .arg("-f")
        .arg(values_file)
        .arg("--set-file")
        .arg(format!("cfgsync.config={cfgsync_file}"))
        .arg("--set-file")
        .arg(format!("scripts.runCfgsyncSh={run_cfgsync_script}"))
        .arg("--set-file")
        .arg(format!("scripts.runNomosNodeSh={run_nomos_node_script}"))
        .arg("--set-file")
        .arg(format!(
            "scripts.runNomosExecutorSh={run_nomos_executor_script}"
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Ok(root) = workspace_root() {
        cmd.current_dir(root);
    }

    let output = cmd
        .output()
        .await
        .map_err(|err| format!("failed to spawn helm install: {err}"))?;

    if std::env::var("K8S_RUNNER_DEBUG").is_ok() {
        println!(
            "[k8s-runner] helm install stdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        println!(
            "[k8s-runner] helm install stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "helm install failed: {}\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ))
    }
}

pub async fn uninstall_release(release: &str, namespace: &str) -> Result<(), String> {
    let mut cmd = Command::new("helm");
    cmd.arg("uninstall")
        .arg(release)
        .arg("--namespace")
        .arg(namespace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    println!("[k8s-runner] issuing `helm uninstall {release}` in namespace `{namespace}`");

    let output = cmd
        .output()
        .await
        .map_err(|err| format!("failed to spawn helm uninstall: {err}"))?;

    if output.status.success() {
        println!(
            "[k8s-runner] helm uninstall {release} completed successfully (namespace `{namespace}`)"
        );
        Ok(())
    } else {
        Err(format!(
            "helm uninstall failed: {}\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ))
    }
}
