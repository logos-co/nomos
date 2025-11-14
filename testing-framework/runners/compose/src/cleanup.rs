use std::{env, path::PathBuf};

use testing_framework_core::scenario::CleanupGuard;

use crate::{cfgsync::CfgsyncServerHandle, compose::compose_down, workspace::ComposeWorkspace};

pub struct RunnerCleanup {
    pub compose_file: PathBuf,
    pub project_name: String,
    pub root: PathBuf,
    workspace: Option<ComposeWorkspace>,
    cfgsync: Option<CfgsyncServerHandle>,
}

impl RunnerCleanup {
    pub fn new(
        compose_file: PathBuf,
        project_name: String,
        root: PathBuf,
        workspace: ComposeWorkspace,
        cfgsync: Option<CfgsyncServerHandle>,
    ) -> Self {
        debug_assert!(
            !compose_file.as_os_str().is_empty() && !project_name.is_empty(),
            "compose cleanup should receive valid identifiers"
        );
        Self {
            compose_file,
            project_name,
            root,
            workspace: Some(workspace),
            cfgsync,
        }
    }

    fn teardown_compose(&self) {
        if let Err(err) = compose_down(&self.compose_file, &self.project_name, &self.root) {
            eprintln!("[compose-runner] docker compose down failed: {err}");
        }
    }
}

impl CleanupGuard for RunnerCleanup {
    fn cleanup(mut self: Box<Self>) {
        let preserve = env::var("COMPOSE_RUNNER_PRESERVE").is_ok()
            || env::var("TESTNET_RUNNER_PRESERVE").is_ok();
        if preserve {
            if let Some(mut handle) = self.cfgsync.take() {
                handle.shutdown();
            }

            if let Some(workspace) = self.workspace.take() {
                let keep = workspace.into_inner().keep();
                eprintln!(
                    "[compose-runner] preserving docker state at {}",
                    keep.display()
                );
            }

            eprintln!("[compose-runner] compose preserve flag set; skipping docker compose down");
            return;
        }

        self.teardown_compose();

        if let Some(mut handle) = self.cfgsync.take() {
            handle.shutdown();
        }
    }
}
