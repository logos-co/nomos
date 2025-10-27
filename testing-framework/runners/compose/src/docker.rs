//! Prototype runner that will orchestrate scenarios via Docker Compose. The
//! current implementation only provides the scaffolding required to plug a
//! compose-based flow in the future.

use std::path::PathBuf;

use futures::FutureExt as _;
use testing_framework_core::scenario::{BoxFuture, ExecutionPlan, RunHandle, Runner};

/// Runner intended to execute scenarios by delegating to docker-compose.
#[derive(Clone, Debug)]
pub struct DockerRunner {
    compose_file: PathBuf,
    project_name: Option<String>,
}

impl DockerRunner {
    #[must_use]
    pub fn new(compose_file: impl Into<PathBuf>) -> Self {
        Self {
            compose_file: compose_file.into(),
            project_name: None,
        }
    }

    #[must_use]
    pub fn with_project_name(mut self, project: impl Into<String>) -> Self {
        self.project_name = Some(project.into());
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DockerRunnerError {
    #[error("docker runner is not implemented yet (compose file: {compose_file:?})")]
    NotImplemented { compose_file: PathBuf },
}

impl Runner for DockerRunner {
    type Error = DockerRunnerError;

    fn run<'a>(
        &'a self,
        _plan: &'a ExecutionPlan,
    ) -> BoxFuture<'a, Result<RunHandle, Self::Error>> {
        async move {
            Err(DockerRunnerError::NotImplemented {
                compose_file: self.compose_file.clone(),
            })
        }
        .boxed()
    }
}
