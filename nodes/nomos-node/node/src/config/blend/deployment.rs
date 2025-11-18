use core::num::NonZeroU64;

use nomos_blend_service::{core::settings::SchedulerSettings, settings::TimingSettings};
use nomos_libp2p::protocol_name::StreamProtocol;
use nomos_utils::math::NonNegativeF64;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    pub core: CoreSettings,
    pub edge: EdgeSettings,
    /// `ß_c`: expected number of blending operations for each locally generated
    /// message.
    pub num_blend_layers: u64,
    pub timing: TimingSettings,
    pub minimum_network_size: NonZeroU64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde_as]
pub struct CoreSettings {
    pub scheduler: SchedulerSettings,
    pub minimum_messages_coefficient: NonZeroU64,
    pub normalization_constant: NonNegativeF64,
    #[serde_as(
        as = "nomos_utils::bounded_duration::MinimalBoundedDuration<1, nomos_utils::bounded_duration::SECOND>"
    )]
    pub protocol_name: StreamProtocol,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EdgeSettings {
    pub protocol_name: StreamProtocol,
}
