use core::num::NonZeroU64;
use std::path::PathBuf;

use nomos_blend_scheduling::message_blend::crypto::SessionCryptographicProcessorSettings;
use serde::{Deserialize, Serialize};

use crate::settings::timing::TimingSettings;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommonSettings {
    pub crypto: SessionCryptographicProcessorSettings,
    pub time: TimingSettings,
    pub minimum_network_size: NonZeroU64,
    pub recovery_path_prefix: PathBuf,
    // TODO: Should not be deserialized. So the whole struct should probably not be deserializable,
    // and the deserialization should happen in the binary directly.
    pub deployment: DeploymentSettings,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeploymentSettings;
