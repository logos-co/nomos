use std::{collections::HashSet, hash::Hash, time::Duration};

use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[serde_with::serde_as]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BootstrapConfig<NodeId>
where
    NodeId: Clone + Eq + Hash,
{
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub prolonged_bootstrap_period: Duration,
    pub force_bootstrap: bool,
    #[serde(default)]
    pub offline_grace_period: OfflineGracePeriodConfig,
    pub ibd: IbdConfig<NodeId>,
}

#[serde_with::serde_as]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OfflineGracePeriodConfig {
    /// Maximum duration a node can be offline before forcing bootstrap mode
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    #[serde(default = "default_offline_grace_period")]
    pub grace_period: Duration,
    /// Interval at which to record the current timestamp and engine state
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    #[serde(default = "default_state_recording_interval")]
    pub state_recording_interval: Duration,
}

/// IBD configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IbdConfig<NodeId>
where
    NodeId: Clone + Eq + Hash,
{
    /// Peers to download blocks from.
    pub peers: HashSet<NodeId>,
    /// Delay before attempting the next download
    /// when no download is needed at the moment from a peer.
    #[serde(default = "default_delay_before_new_download")]
    pub delay_before_new_download: Duration,
}

const fn default_offline_grace_period() -> Duration {
    Duration::from_secs(20 * 60)
}

const fn default_state_recording_interval() -> Duration {
    Duration::from_secs(60)
}

const fn default_delay_before_new_download() -> Duration {
    Duration::from_secs(10)
}

impl Default for OfflineGracePeriodConfig {
    fn default() -> Self {
        Self {
            grace_period: default_offline_grace_period(),
            state_recording_interval: default_state_recording_interval(),
        }
    }
}
