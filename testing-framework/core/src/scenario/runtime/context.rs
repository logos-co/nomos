use std::{sync::Arc, time::Duration};

use super::{block_feed::BlockFeed, metrics::Metrics, node_clients::ClusterClient};
use crate::{
    nodes::ApiClient,
    scenario::{NodeClients, NodeControlHandle},
    topology::{GeneratedTopology, Topology, configs::wallet::WalletAccount},
};

pub struct RunContext {
    descriptors: GeneratedTopology,
    cluster: Option<Topology>,
    node_clients: NodeClients,
    metrics: RunMetrics,
    telemetry: Metrics,
    block_feed: BlockFeed,
    node_control: Option<Arc<dyn NodeControlHandle>>,
}

impl RunContext {
    /// Builds a run context, clamping the requested duration so we always run
    /// for at least a couple of slot lengths (or a fallback window if slots are
    /// unknown).
    #[must_use]
    pub fn new(
        descriptors: GeneratedTopology,
        cluster: Option<Topology>,
        node_clients: NodeClients,
        run_duration: Duration,
        telemetry: Metrics,
        block_feed: BlockFeed,
        node_control: Option<Arc<dyn NodeControlHandle>>,
    ) -> Self {
        let metrics = RunMetrics::new(&descriptors, run_duration);

        Self {
            descriptors,
            cluster,
            node_clients,
            metrics,
            telemetry,
            block_feed,
            node_control,
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
    pub const fn node_clients(&self) -> &NodeClients {
        &self.node_clients
    }

    #[must_use]
    pub fn random_node_client(&self) -> Option<&ApiClient> {
        self.node_clients.any_client()
    }

    #[must_use]
    pub fn block_feed(&self) -> BlockFeed {
        self.block_feed.clone()
    }

    #[must_use]
    pub fn wallet_accounts(&self) -> &[WalletAccount] {
        self.descriptors.wallet_accounts()
    }

    #[must_use]
    pub const fn telemetry(&self) -> &Metrics {
        &self.telemetry
    }

    #[must_use]
    pub const fn run_duration(&self) -> Duration {
        self.metrics.run_duration()
    }

    #[must_use]
    pub const fn expected_blocks(&self) -> u64 {
        self.metrics.expected_consensus_blocks()
    }

    #[must_use]
    pub const fn run_metrics(&self) -> RunMetrics {
        self.metrics
    }

    #[must_use]
    pub fn node_control(&self) -> Option<Arc<dyn NodeControlHandle>> {
        self.node_control.clone()
    }

    #[must_use]
    pub const fn cluster_client(&self) -> ClusterClient<'_> {
        self.node_clients.cluster_client()
    }
}

/// Handle returned by the runner to control the lifecycle of the run.
pub struct RunHandle {
    run_context: Arc<RunContext>,
    cleanup_guard: Option<Box<dyn CleanupGuard>>,
}

impl Drop for RunHandle {
    fn drop(&mut self) {
        if let Some(guard) = self.cleanup_guard.take() {
            guard.cleanup();
        }
    }
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
    pub(crate) fn from_shared(
        context: Arc<RunContext>,
        cleanup_guard: Option<Box<dyn CleanupGuard>>,
    ) -> Self {
        Self {
            run_context: context,
            cleanup_guard,
        }
    }

    #[must_use]
    pub fn context(&self) -> &RunContext {
        &self.run_context
    }
}

#[derive(Clone, Copy)]
pub struct RunMetrics {
    run_duration: Duration,
    expected_blocks: u64,
    block_interval_hint: Option<Duration>,
}

impl RunMetrics {
    #[must_use]
    pub fn new(descriptors: &GeneratedTopology, run_duration: Duration) -> Self {
        Self::from_topology(descriptors, run_duration)
    }

    #[must_use]
    pub fn from_topology(descriptors: &GeneratedTopology, run_duration: Duration) -> Self {
        let slot_duration = descriptors.slot_duration();

        let active_slot_coeff = descriptors.config().consensus_params.active_slot_coeff;
        let expected_blocks =
            calculate_expected_blocks(run_duration, slot_duration, active_slot_coeff);

        let block_interval_hint =
            slot_duration.map(|duration| duration.mul_f64(active_slot_coeff.clamp(0.0, 1.0)));

        Self {
            run_duration,
            expected_blocks,
            block_interval_hint,
        }
    }

    #[must_use]
    pub const fn run_duration(&self) -> Duration {
        self.run_duration
    }

    #[must_use]
    pub const fn expected_consensus_blocks(&self) -> u64 {
        self.expected_blocks
    }

    #[must_use]
    pub const fn block_interval_hint(&self) -> Option<Duration> {
        self.block_interval_hint
    }
}

pub trait CleanupGuard: Send {
    fn cleanup(self: Box<Self>);
}

/// Computes the minimum duration we’ll allow for a scenario run so that the
/// scheduler can observe a few block opportunities even if the caller
/// requested an extremely short window.
fn calculate_expected_blocks(
    run_duration: Duration,
    slot_duration: Option<Duration>,
    active_slot_coeff: f64,
) -> u64 {
    let Some(slot_duration) = slot_duration else {
        return 0;
    };
    let slot_secs = slot_duration.as_secs_f64();
    let run_secs = run_duration.as_secs_f64();
    let expected = run_secs / slot_secs * active_slot_coeff;

    expected.ceil().clamp(0.0, u64::MAX as f64) as u64
}
