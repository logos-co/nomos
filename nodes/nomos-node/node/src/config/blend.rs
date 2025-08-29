use nomos_blend_service::settings::NetworkSettings;
use overwatch::services::ServiceData;
use serde::{Deserialize, Serialize};

use crate::{BlendCoreService, BlendEdgeService, BlendService};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlendConfig(<BlendCoreService as ServiceData>::Settings);

impl BlendConfig {
    #[must_use]
    pub const fn new(core: <BlendCoreService as ServiceData>::Settings) -> Self {
        Self(core)
    }

    fn edge(&self) -> <BlendEdgeService as ServiceData>::Settings {
        nomos_blend_service::edge::settings::BlendConfig {
            backend: nomos_blend_service::edge::backends::libp2p::Libp2pBlendBackendSettings {
                node_key: self.0.backend.node_key.clone(),
                // TODO: Allow for edge service settings to be included here.
                max_dial_attempts_per_peer_per_message: 3
                    .try_into()
                    .expect("Max dial attempts per peer per message cannot be zero."),
                protocol_name: self.0.backend.protocol_name.clone(),
                // TODO: Allow for edge service settings to be included here.
                replication_factor: 4
                    .try_into()
                    .expect("Edge message replication factor cannot be zero."),
            },
            crypto: self.0.crypto.clone(),
            time: self.0.time.clone(),
            membership: self.0.membership.clone(),
            minimum_network_size: self.0.minimum_network_size,
        }
    }

    pub const fn get_mut(&mut self) -> &mut <BlendCoreService as ServiceData>::Settings {
        &mut self.0
    }
}

impl From<BlendConfig>
    for (
        <BlendService as ServiceData>::Settings,
        <BlendCoreService as ServiceData>::Settings,
        <BlendEdgeService as ServiceData>::Settings,
    )
{
    fn from(config: BlendConfig) -> Self {
        let edge = config.edge();
        (
            NetworkSettings {
                membership: config.0.membership.clone(),
                minimal_network_size: config.0.minimum_network_size,
            },
            config.0,
            edge,
        )
    }
}
