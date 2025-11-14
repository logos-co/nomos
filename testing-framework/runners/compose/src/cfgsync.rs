use std::{net::Ipv4Addr, path::Path, sync::Arc};

use anyhow::Context as _;
use axum::serve;
use cfgsync::{
    repo::ConfigRepo,
    server::{CfgSyncConfig as ServerCfgSyncConfig, cfgsync_app},
};
use testing_framework_core::{
    scenario::cfgsync::{apply_topology_overrides, load_cfgsync_template, write_cfgsync_template},
    topology::GeneratedTopology,
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

#[derive(Debug)]
pub struct CfgsyncServerHandle {
    shutdown: Option<oneshot::Sender<()>>,
    pub join: JoinHandle<()>,
}

impl CfgsyncServerHandle {
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        self.join.abort();
    }
}

pub fn update_cfgsync_config(
    path: &Path,
    topology: &GeneratedTopology,
    use_kzg_mount: bool,
) -> anyhow::Result<()> {
    let mut cfg = load_cfgsync_template(path)?;
    apply_topology_overrides(&mut cfg, topology, use_kzg_mount);
    write_cfgsync_template(path, &cfg)?;
    Ok(())
}

pub async fn start_cfgsync_server(
    cfgsync_path: &Path,
    port: u16,
) -> anyhow::Result<CfgsyncServerHandle> {
    let cfg_path = cfgsync_path.to_path_buf();
    let config = ServerCfgSyncConfig::load_from_file(&cfg_path)
        .map_err(|err| anyhow::anyhow!("loading cfgsync config: {err}"))?;
    let repo: Arc<ConfigRepo> = config.into();

    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))
        .await
        .context("binding cfgsync listener")?;

    let cfgsync_router = cfgsync_app(repo);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (ready_tx, ready_rx) = oneshot::channel();

    let join = tokio::spawn(async move {
        let server =
            serve(listener, cfgsync_router.into_make_service()).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
        let _ = ready_tx.send(());
        if let Err(err) = server.await {
            eprintln!("[compose-runner] cfgsync server error: {err}");
        }
    });

    ready_rx
        .await
        .context("waiting for cfgsync server to become ready")?;

    Ok(CfgsyncServerHandle {
        shutdown: Some(shutdown_tx),
        join,
    })
}
