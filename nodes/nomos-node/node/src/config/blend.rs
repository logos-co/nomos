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
            },
            crypto: self.0.crypto.clone(),
            time: self.0.time.clone(),
            membership: self.0.membership.clone(),
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
        ((), config.0, edge)
    }
}
