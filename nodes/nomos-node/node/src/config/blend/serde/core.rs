use core::{num::NonZeroU64, ops::RangeInclusive, time::Duration};

use nomos_libp2p::Multiaddr;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub backend: BackendSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde_as]
pub struct BackendSettings {
    pub listening_address: Multiaddr,
    pub core_peering_degree: RangeInclusive<u64>,
    #[serde_as(
        as = "nomos_utils::bounded_duration::MinimalBoundedDuration<1, nomos_utils::bounded_duration::SECOND>"
    )]
    pub edge_node_connection_timeout: Duration,
    pub max_edge_node_incoming_connections: u64,
    pub max_dial_attempts_per_peer: NonZeroU64,
}
