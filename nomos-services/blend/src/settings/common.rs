use core::num::NonZeroU64;

use nomos_blend_scheduling::message_blend::crypto::SessionCryptographicProcessorSettings;
use serde::{Deserialize, Serialize};

use crate::settings::timing::TimingSettings;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommonSettings {
    pub crypto: SessionCryptographicProcessorSettings,
    pub time: TimingSettings,
    pub minimum_network_size: NonZeroU64,
}
