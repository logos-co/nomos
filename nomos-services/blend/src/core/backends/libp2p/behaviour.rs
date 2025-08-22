use std::num::NonZeroU64;

use libp2p::{allow_block_list::BlockedPeers, connection_limits::ConnectionLimits, PeerId};
use nomos_blend_network::core::with_core::behaviour::ObservationWindowTokioIntervalProvider;
use nomos_libp2p::NetworkBehaviour;

use crate::core::{backends::libp2p::Libp2pBlendBackendSettings, settings::BlendConfig};

#[derive(NetworkBehaviour)]
pub(super) struct BlendBehaviour {
    pub(super) blend:
        nomos_blend_network::core::NetworkBehaviour<ObservationWindowTokioIntervalProvider>,
    pub(super) limits: libp2p::connection_limits::Behaviour,
    pub(super) blocked_peers: libp2p::allow_block_list::Behaviour<BlockedPeers>,
}

impl BlendBehaviour {
    pub(super) fn new(config: &BlendConfig<Libp2pBlendBackendSettings, PeerId>) -> Self {
        let observation_window_interval_provider = ObservationWindowTokioIntervalProvider {
            blending_ops_per_message: config.crypto.num_blend_layers,
            maximal_delay_rounds: config.scheduler.delayer.maximum_release_delay_in_rounds,
            membership_size: NonZeroU64::try_from(config.membership().size() as u64)
                .expect("Membership size cannot be zero."),
            minimum_messages_coefficient: config.backend.minimum_messages_coefficient,
            normalization_constant: config.backend.normalization_constant,
            round_duration_seconds: config
                .time
                .round_duration
                .as_secs()
                .try_into()
                .expect("Round duration cannot be zero."),
            rounds_per_observation_window: config.time.rounds_per_observation_window,
        };
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
                config.backend.protocol_name.clone().into_inner(),
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
