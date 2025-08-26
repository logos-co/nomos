use core::num::NonZeroU64;

use nomos_blend_service::settings::NetworkSettings;
use overwatch::services::ServiceData;
use serde::{Deserialize, Serialize};

use crate::{BlendCoreService, BlendEdgeService, BlendService};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlendConfig {
    #[serde(flatten)]
    core: <BlendCoreService as ServiceData>::Settings,
    minimum_network_size: NonZeroU64,
}

impl BlendConfig {
    #[must_use]
    pub const fn new(core: <BlendCoreService as ServiceData>::Settings) -> Self {
        Self {
            core,
            minimum_network_size,
        }
    }

    fn edge(&self) -> <BlendEdgeService as ServiceData>::Settings {
        nomos_blend_service::edge::settings::BlendConfig {
            backend: nomos_blend_service::edge::backends::libp2p::Libp2pBlendBackendSettings {
                node_key: self.core.backend.node_key.clone(),
                // TODO: Allow for edge service settings to be included here.
                max_dial_attempts_per_peer_per_message: 3
                    .try_into()
                    .expect("Max dial attempts per peer per message cannot be zero."),
                protocol_name: self.core.backend.protocol_name.clone(),
            },
            crypto: self.core.crypto.clone(),
            time: self.core.time.clone(),
            membership: self.core.membership.clone(),
        }
    }

    pub const fn get_mut(&mut self) -> &mut <BlendCoreService as ServiceData>::Settings {
        &mut self.core
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
                membership: config.core.membership.clone(),
                minimal_network_size: config.minimum_network_size,
            },
            config.core,
            edge,
        )
    }
}
