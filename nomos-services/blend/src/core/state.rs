use core::{convert::Infallible, hash::Hash, marker::PhantomData};
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{core::settings::BlendConfig, message::ProcessedMessage};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Recovery state for Blend core service.
pub struct ServiceState<BroadcastSettings> {
    /// The last session that was saved.
    last_seen_session: u64,
    /// The last value for the core quota allowance for the session that is
    /// tracked.
    spent_core_quota: u64,
    #[serde(bound(deserialize = "BroadcastSettings: Deserialize<'de> + Eq + core::hash::Hash"))]
    unsent_processed_messages: HashSet<ProcessedMessage<BroadcastSettings>>,
}

impl<BroadcastSettings> ServiceState<BroadcastSettings> {
    pub fn with_session(session: u64) -> Self {
        Self {
            last_seen_session: session,
            spent_core_quota: 0,
            unsent_processed_messages: HashSet::new(),
        }
    }

    pub const fn last_seen_session(&self) -> u64 {
        self.last_seen_session
    }

    pub const fn spent_core_quota(&self) -> u64 {
        self.spent_core_quota
    }

    pub const fn unsent_processed_messages(&self) -> &HashSet<ProcessedMessage<BroadcastSettings>> {
        &self.unsent_processed_messages
    }

    pub const fn by_consuming_core_quota(mut self, amount: u64) -> Self {
        self.spent_core_quota = self
            .spent_core_quota
            .checked_add(amount)
            .expect("Spent core quota addition overflow.");
        self
    }
}

impl<BroadcastSettings> ServiceState<BroadcastSettings>
where
    BroadcastSettings: Eq + Hash,
{
    pub fn by_adding_unsent_message(
        mut self,
        message: ProcessedMessage<BroadcastSettings>,
    ) -> Result<Self, Self> {
        if self.unsent_processed_messages.insert(message) {
            Ok(self)
        } else {
            Err(self)
        }
    }

    pub fn remove_sent_message(
        &mut self,
        message: &ProcessedMessage<BroadcastSettings>,
    ) -> Result<(), ()> {
        if self.unsent_processed_messages.remove(message) {
            Ok(())
        } else {
            Err(())
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RecoveryServiceState<BackendSettings, BroadcastSettings> {
    #[serde(bound(deserialize = "BroadcastSettings: Deserialize<'de> + Eq + core::hash::Hash"))]
    pub service_state: Option<ServiceState<BroadcastSettings>>,
    _phantom: PhantomData<BackendSettings>,
}

impl<BackendSettings, BroadcastSettings> From<ServiceState<BroadcastSettings>>
    for RecoveryServiceState<BackendSettings, BroadcastSettings>
{
    fn from(value: ServiceState<BroadcastSettings>) -> Self {
        Self {
            _phantom: PhantomData,
            service_state: Some(value),
        }
    }
}

impl<BackendSettings, BroadcastSettings> overwatch::services::state::ServiceState
    for RecoveryServiceState<BackendSettings, BroadcastSettings>
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
