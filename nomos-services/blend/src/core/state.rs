use core::{convert::Infallible, marker::PhantomData};

use serde::{Deserialize, Serialize};

use crate::core::settings::BlendConfig;

#[derive(Clone, Serialize, Deserialize)]
pub struct InnerState {
    pub last_seen_session: u64,
    pub remaining_core_quota: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ServiceState<BackendSettings> {
    pub inner_state: Option<InnerState>,
    _phantom: PhantomData<BackendSettings>,
}

impl<BackendSettings> overwatch::services::state::ServiceState for ServiceState<BackendSettings> {
    type Error = Infallible;
    type Settings = BlendConfig<BackendSettings>;

    fn from_settings(_: &Self::Settings) -> Result<Self, Self::Error> {
        Ok(Self {
            _phantom: PhantomData,
            inner_state: None,
        })
    }
}
