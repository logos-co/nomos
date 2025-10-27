use core::{convert::Infallible, marker::PhantomData};

use serde::{Deserialize, Serialize};

use crate::core::settings::BlendConfig;

#[derive(Clone, Serialize, Deserialize)]
pub struct ServiceState {
    pub last_seen_session: u64,
    pub spent_core_quota: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RecoveryServiceState<BackendSettings> {
    pub service_state: Option<ServiceState>,
    _phantom: PhantomData<BackendSettings>,
}

impl<BackendSettings> overwatch::services::state::ServiceState
    for RecoveryServiceState<BackendSettings>
{
    type Error = Infallible;
    type Settings = BlendConfig<BackendSettings>;

    fn from_settings(_: &Self::Settings) -> Result<Self, Self::Error> {
        Ok(Self {
            _phantom: PhantomData,
            service_state: None,
        })
    }
}
