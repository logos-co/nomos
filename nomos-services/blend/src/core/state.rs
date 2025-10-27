use core::{convert::Infallible, marker::PhantomData};

use serde::{Deserialize, Serialize};

use crate::core::settings::BlendConfig;

#[derive(Clone, Serialize, Deserialize)]
pub struct ServiceState {
    last_seen_session: u64,
    spent_core_quota: u64,
}

impl ServiceState {
    pub const fn new(last_seen_session: u64, spent_core_quota: u64) -> Self {
        Self {
            last_seen_session,
            spent_core_quota,
        }
    }

    pub const fn with_session(session: u64) -> Self {
        Self::new(session, 0)
    }

    pub const fn last_seen_session(&self) -> u64 {
        self.last_seen_session
    }

    pub const fn spent_core_quota(&self) -> u64 {
        self.spent_core_quota
    }

    pub const fn with_consumed_core_quota(mut self, amount: u64) -> Self {
        self.spent_core_quota = self
            .spent_core_quota
            .checked_add(amount)
            .expect("Spent core quota addition overflow.");
        self
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RecoveryServiceState<BackendSettings> {
    pub service_state: Option<ServiceState>,
    _phantom: PhantomData<BackendSettings>,
}

impl<BackendSettings> From<ServiceState> for RecoveryServiceState<BackendSettings> {
    fn from(value: ServiceState) -> Self {
        Self {
            _phantom: PhantomData,
            service_state: Some(value),
        }
    }
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
