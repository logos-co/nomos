use core::{num::NonZeroU64, ops::RangeInclusive, time::Duration};

use nomos_blend_service::core::settings::ZkSettings;
use nomos_libp2p::Multiaddr;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub backend: BackendConfig,
    pub zk: ZkSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde_as]
pub struct BackendConfig {
    pub listening_address: Multiaddr,
    pub core_peering_degree: RangeInclusive<u64>,
    #[serde_as(
        as = "nomos_utils::bounded_duration::MinimalBoundedDuration<1, nomos_utils::bounded_duration::SECOND>"
    )]
    pub edge_node_connection_timeout: Duration,
    pub max_edge_node_incoming_connections: u64,
    pub max_dial_attempts_per_peer: NonZeroU64,
}
