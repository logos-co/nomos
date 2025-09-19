use core::num::NonZeroU64;

use nomos_blend_scheduling::message_blend::CryptographicProcessorSettings;
use serde::{Deserialize, Serialize};

use crate::settings::TimingSettings;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlendConfig<BackendSettings> {
    pub backend: BackendSettings,
    pub crypto: CryptographicProcessorSettings,
    pub time: TimingSettings,
    pub minimum_network_size: NonZeroU64,
}
