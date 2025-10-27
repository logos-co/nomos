use std::{sync::Arc, time::Duration};

use chain_service::CryptarchiaInfo;
use common_http_client::Error as HttpApiError;
use nomos_core::{block::Block, header::HeaderId, mantle::SignedMantleTx};

use super::{metrics::Metrics, plan::BoxFuture as ScenarioBoxFuture};
use crate::topology::{GeneratedNodeConfig, GeneratedTopology, Topology};

/// Context available to workloads and expectations once the runner has
/// materialised the topology.
pub struct RunContext {
    descriptors: GeneratedTopology,
    cluster: Option<Topology>,
    validator_clients: Vec<NodeClientHandle>,
    metrics: Metrics,
    run_duration: Duration,
}

impl RunContext {
    #[must_use]
    pub fn new(
        descriptors: GeneratedTopology,
        cluster: Option<Topology>,
        validator_clients: Vec<NodeClientHandle>,
        metrics: Metrics,
        run_duration: Duration,
    ) -> Self {
        debug_assert!(
            validator_clients.len() <= descriptors.validators().len(),
            "validator clients should not exceed descriptor count"
        );
        Self {
            descriptors,
            cluster,
            validator_clients,
            metrics,
            run_duration,
        }
    }

    #[must_use]
    pub const fn descriptors(&self) -> &GeneratedTopology {
        &self.descriptors
    }

    #[must_use]
    pub const fn topology(&self) -> Option<&Topology> {
        self.cluster.as_ref()
    }

    #[must_use]
    pub fn validators(&self) -> &[NodeClientHandle] {
        debug_assert!(
            self.validator_clients.len() <= self.descriptors.validators().len(),
            "validator clients should not exceed descriptor count"
        );
        &self.validator_clients
    }

    #[must_use]
    pub const fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    #[must_use]
    pub const fn run_duration(&self) -> Duration {
        self.run_duration
    }
}

/// Handle returned by the runner to control the lifecycle of the run.
pub struct RunHandle {
    run_context: Arc<RunContext>,
    cleanup_guard: Option<Box<dyn CleanupGuard>>,
}

impl RunHandle {
    #[must_use]
    pub fn new(context: RunContext, cleanup_guard: Option<Box<dyn CleanupGuard>>) -> Self {
        Self {
            run_context: Arc::new(context),
            cleanup_guard,
        }
    }

    #[must_use]
    pub fn from_arc(context: Arc<RunContext>) -> Self {
        Self {
            run_context: context,
            cleanup_guard: None,
        }
    }

    #[must_use]
    pub fn context(&self) -> &RunContext {
        &self.run_context
    }
}

impl Drop for RunHandle {
    fn drop(&mut self) {
        if let Some(guard) = self.cleanup_guard.take() {
            guard.cleanup();
        }
    }
}

/// Wrapper for node clients; executors expose their control surface directly
/// via workloads so no dedicated executor handle is required.
#[derive(Clone)]
pub struct NodeClientHandle {
    inner: Arc<dyn NodeClient>,
}

impl NodeClientHandle {
    pub fn new(inner: Arc<dyn NodeClient>) -> Self {
        Self { inner }
    }

    #[must_use]
    pub fn client(&self) -> &dyn NodeClient {
        self.inner.as_ref()
    }

    #[must_use]
    pub fn descriptor(&self) -> &GeneratedNodeConfig {
        self.inner.descriptor()
    }
}

pub trait NodeClient: Send + Sync {
    fn descriptor(&self) -> &GeneratedNodeConfig;
    fn submit_transaction(
        &self,
        tx: SignedMantleTx,
    ) -> ScenarioBoxFuture<'_, Result<(), ClientError>>;
    fn consensus_info(&self) -> ScenarioBoxFuture<'_, Result<CryptarchiaInfo, ClientError>>;
    fn get_block(
        &self,
        id: HeaderId,
    ) -> ScenarioBoxFuture<'_, Result<Option<Block<SignedMantleTx>>, ClientError>>;
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("http api error: {0}")]
    Http(String),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
}

impl From<HttpApiError> for ClientError {
    fn from(value: HttpApiError) -> Self {
        Self::Http(value.to_string())
    }
}

pub trait CleanupGuard: Send {
    fn cleanup(self: Box<Self>);
}
