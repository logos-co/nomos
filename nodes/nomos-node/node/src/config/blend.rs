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
        (config.proxy(), config.core(), config.edge())
    }
}
