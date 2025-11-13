use nomos_libp2p::Multiaddr;
use overwatch::services::ServiceData;
use serde::{Deserialize, Serialize};

use crate::{BlendCoreService, BlendEdgeService, BlendService};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlendConfig(<BlendService as ServiceData>::Settings);

impl BlendConfig {
    #[must_use]
    pub const fn new(settings: <BlendService as ServiceData>::Settings) -> Self {
        Self(settings)
    }

    fn core(&self) -> <BlendCoreService as ServiceData>::Settings {
        self.0.clone().into()
    }

    fn edge(&self) -> <BlendEdgeService as ServiceData>::Settings {
        self.0.clone().into()
    }

    pub fn set_listening_address(&mut self, listening_address: Multiaddr) {
        self.0.core.backend.listening_address = listening_address;
    }

    pub const fn set_blend_layers(&mut self, layers: u64) {
        self.0.common.crypto.num_blend_layers = layers;
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
        (config.0.clone(), config.core(), config.edge())
    }
}
