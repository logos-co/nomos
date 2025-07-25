use std::num::NonZeroU64;

use libp2p::{allow_block_list::BlockedPeers, connection_limits::ConnectionLimits, PeerId};
use nomos_blend_network::core::ObservationWindowTokioIntervalProvider;
use nomos_blend_scheduling::message_blend::crypto::CryptographicProcessor;
use nomos_libp2p::NetworkBehaviour;

use crate::{backends::libp2p::Libp2pBlendBackendSettings, BlendConfig};

#[derive(NetworkBehaviour)]
pub(super) struct BlendBehaviour<Rng> {
    pub(super) blend: nomos_blend_network::core::Behaviour<ObservationWindowTokioIntervalProvider>,
    pub(super) limits: libp2p::connection_limits::Behaviour,
    pub(super) blocked_peers: libp2p::allow_block_list::Behaviour<BlockedPeers>,
}

impl<Rng> BlendBehaviour<Rng> {
    pub(super) fn new(config: &BlendConfig<Libp2pBlendBackendSettings, PeerId>, rng: Rng) -> Self {
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
        Self {
            blend: nomos_blend_network::core::Behaviour::new(
                observation_window_interval_provider,
                Some(config.membership()),
                config.backend.edge_node_connection_timeout,
                CryptographicProcessor::new(config.crypto.clone(), config.membership(), rng),
            ),
            limits: libp2p::connection_limits::Behaviour::new(
                ConnectionLimits::default()
                    .with_max_established(Some(config.backend.max_peering_degree))
                    .with_max_established_incoming(Some(config.backend.max_peering_degree))
                    .with_max_established_outgoing(Some(config.backend.max_peering_degree))
                    // Blend protocol restricts the number of connections per peer to 1.
                    .with_max_established_per_peer(Some(1)),
            ),
            blocked_peers: libp2p::allow_block_list::Behaviour::default(),
        }
    }
}
