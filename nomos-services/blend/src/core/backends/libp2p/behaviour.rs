use libp2p::{allow_block_list::BlockedPeers, connection_limits::ConnectionLimits, PeerId};
use nomos_libp2p::NetworkBehaviour;

use crate::core::{backends::libp2p::Libp2pBlendBackendSettings, settings::BlendConfig};

#[derive(NetworkBehaviour)]
pub(super) struct BlendBehaviour<ObservationWindowProvider> {
    pub(super) blend: nomos_blend_network::core::NetworkBehaviour<ObservationWindowProvider>,
    pub(super) limits: libp2p::connection_limits::Behaviour,
    pub(super) blocked_peers: libp2p::allow_block_list::Behaviour<BlockedPeers>,
}

impl<ObservationWindowProvider> BlendBehaviour<ObservationWindowProvider>
where
    ObservationWindowProvider: for<'c> From<&'c BlendConfig<Libp2pBlendBackendSettings, PeerId>>,
{
    pub(super) fn new(config: &BlendConfig<Libp2pBlendBackendSettings, PeerId>) -> Self {
        let observation_window_interval_provider = ObservationWindowProvider::from(config);
        let minimum_core_healthy_peering_degree =
            *config.backend.core_peering_degree.start() as usize;
        let maximum_core_peering_degree = *config.backend.core_peering_degree.end() as usize;
        let maximum_edge_incoming_connections =
            config.backend.max_edge_node_incoming_connections as usize;
        Self {
            blend: nomos_blend_network::core::NetworkBehaviour::new(
                &nomos_blend_network::core::Config {
                    with_core: nomos_blend_network::core::with_core::behaviour::Config {
                        peering_degree: minimum_core_healthy_peering_degree
                            ..=maximum_core_peering_degree,
                    },
                    with_edge: nomos_blend_network::core::with_edge::behaviour::Config {
                        connection_timeout: config.backend.edge_node_connection_timeout,
                        max_incoming_connections: maximum_edge_incoming_connections,
                    },
                },
                observation_window_interval_provider,
                Some(config.membership()),
                config.backend.peer_id(),
            ),
            limits: libp2p::connection_limits::Behaviour::new(
                ConnectionLimits::default()
                    // Max established = max core peering degree + max edge incoming connections.
                    .with_max_established(Some(
                        maximum_core_peering_degree
                            .saturating_add(maximum_edge_incoming_connections)
                            as u32,
                    ))
                    // Max established incoming = max established.
                    .with_max_established_incoming(Some(
                        maximum_core_peering_degree
                            .saturating_add(maximum_edge_incoming_connections)
                            as u32,
                    ))
                    // Max established outgoing = max core peering degree.
                    .with_max_established_outgoing(Some(maximum_core_peering_degree as u32)),
            ),
            blocked_peers: libp2p::allow_block_list::Behaviour::default(),
        }
    }
}
