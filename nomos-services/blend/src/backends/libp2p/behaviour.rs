use std::num::{NonZeroU64, NonZeroUsize};

use libp2p::{allow_block_list::BlockedPeers, connection_limits::ConnectionLimits, PeerId};
use nomos_blend_network::ObservationWindowTokioIntervalProvider;
use nomos_libp2p::NetworkBehaviour;

use crate::{backends::libp2p::Libp2pBlendBackendSettings, BlendConfig};

#[derive(NetworkBehaviour)]
pub(super) struct BlendBehaviour {
    pub(super) blend: nomos_blend_network::Behaviour<ObservationWindowTokioIntervalProvider>,
    pub(super) limits: libp2p::connection_limits::Behaviour,
    pub(super) blocked_peers: libp2p::allow_block_list::Behaviour<BlockedPeers>,
}

impl BlendBehaviour {
    pub(super) fn new(config: &BlendConfig<Libp2pBlendBackendSettings, PeerId>) -> Self {
        let observation_window_interval_provider = ObservationWindowTokioIntervalProvider {
            blending_ops_per_message: NonZeroU64::try_from(
                config
                    .message_blend
                    .cryptographic_processor
                    .num_blend_layers as u64,
            )
            .unwrap(),
            maximal_delay_seconds: NonZeroU64::try_from(
                config.message_blend.temporal_processor.max_delay.as_secs(),
            )
            .unwrap(),
            membership_size: NonZeroUsize::try_from(config.membership().size()).unwrap(),
            minimum_messages_coefficient: NonZeroUsize::try_from(
                config.message_blend.minimum_messages_coefficient,
            )
            .unwrap(),
            normalization_constant: config.message_blend.normalization_constant,
            round_duration_seconds: NonZeroU64::try_from(
                config.timing_settings.round_duration.as_secs(),
            )
            .unwrap(),
            rounds_per_observation_window: config.timing_settings.rounds_per_observation_window,
        };
        Self {
            blend: nomos_blend_network::Behaviour::new(
                &nomos_blend_network::Config {
                    // TODO: This should be as (ROUNDS_IN_SESSION + BUFFER) * MAX_HOPS,
                    // once session and round mechanisms are implemented.
                    // https://www.notion.so/Blend-Protocol-Version-1-PENDING-MIGRATION-1c48f96fb65c809494efe63019a5ebfb?source=copy_link#2088f96fb65c80be9057c6b4ce6b7023
                    seen_message_cache_size: 1_944_000,
                },
                observation_window_interval_provider,
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
