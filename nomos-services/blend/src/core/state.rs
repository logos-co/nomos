use core::{convert::Infallible, marker::PhantomData};

use serde::{Deserialize, Serialize};

use crate::core::settings::BlendConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Recovery state for Blend core service.
pub struct ServiceState {
    /// The last session that was saved.
    last_seen_session: u64,
    /// The last value for the core quota allowance for the session that is
    /// tracked.
    spent_core_quota: u64,
}

impl ServiceState {
    pub const fn with_session(session: u64) -> Self {
        Self {
            last_seen_session: session,
            spent_core_quota: 0,
        }
    }

    pub const fn last_seen_session(&self) -> u64 {
        self.last_seen_session
    }

    pub const fn spent_core_quota(&self) -> u64 {
        self.spent_core_quota
    }

    pub const fn by_consuming_core_quota(mut self, amount: u64) -> Self {
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
