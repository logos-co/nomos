mod serde {
    use std::collections::HashSet;

    use serde::{Deserialize, Serialize};

    use crate::{
        core::state::{recovery_state::RecoveryServiceState, service::ServiceState},
        message::ProcessedMessage,
    };

    #[derive(Clone, Serialize, Deserialize)]
    pub struct SerializableServiceState<BroadcastSettings> {
        /// The last session that was saved.
        last_seen_session: u64,
        /// The last value for the core quota allowance for the session that is
        /// tracked.
        spent_core_quota: u64,
        #[serde(bound(
            deserialize = "BroadcastSettings: Deserialize<'de> + Eq + core::hash::Hash"
        ))]
        unsent_processed_messages: HashSet<ProcessedMessage<BroadcastSettings>>,
    }

    impl<BroadcastSettings> SerializableServiceState<BroadcastSettings> {
        pub fn into_state_with_state_updater<BackendSettings>(
            self,
            state_updater: overwatch::services::state::StateUpdater<
                Option<RecoveryServiceState<BackendSettings, BroadcastSettings>>,
            >,
        ) -> ServiceState<BackendSettings, BroadcastSettings> {
            ServiceState::new(
                self.last_seen_session,
                self.spent_core_quota,
                self.unsent_processed_messages,
                state_updater,
            )
        }
    }

    impl<BackendSettings, BroadcastSettings> From<ServiceState<BackendSettings, BroadcastSettings>>
        for SerializableServiceState<BroadcastSettings>
    {
        fn from(value: ServiceState<BackendSettings, BroadcastSettings>) -> Self {
            let (last_seen_session, spent_core_quota, unsent_processed_messages, _) =
                value.into_components();
            Self {
                last_seen_session,
                spent_core_quota,
                unsent_processed_messages,
            }
        }
    }
}

pub use self::service::ServiceState;
mod service {
    use core::{
        fmt::{self, Debug, Formatter},
        hash::Hash,
    };
    use std::collections::HashSet;

    use crate::{
        core::state::{recovery_state::RecoveryServiceState, state_updater::StateUpdater},
        message::ProcessedMessage,
    };

    #[derive(Clone)]
    pub struct ServiceState<BackendSettings, BroadcastSettings> {
        last_seen_session: u64,
        spent_core_quota: u64,
        unsent_processed_messages: HashSet<ProcessedMessage<BroadcastSettings>>,
        state_updater: overwatch::services::state::StateUpdater<
            Option<RecoveryServiceState<BackendSettings, BroadcastSettings>>,
        >,
    }

    impl<BackendSettings, BroadcastSettings> Debug for ServiceState<BackendSettings, BroadcastSettings>
    where
        BroadcastSettings: Debug,
    {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            f.debug_struct("ServiceState")
                .field("last_seen_session", &self.last_seen_session)
                .field("spent_core_quota", &self.spent_core_quota)
                .field("unsent_processed_messages", &self.unsent_processed_messages)
                .finish_non_exhaustive()
        }
    }

    impl<BackendSettings, BroadcastSettings> ServiceState<BackendSettings, BroadcastSettings> {
        pub(super) const fn new(
            last_seen_session: u64,
            spent_core_quota: u64,
            unsent_processed_messages: HashSet<ProcessedMessage<BroadcastSettings>>,
            state_updater: overwatch::services::state::StateUpdater<
                Option<RecoveryServiceState<BackendSettings, BroadcastSettings>>,
            >,
        ) -> Self {
            Self {
                last_seen_session,
                spent_core_quota,
                unsent_processed_messages,
                state_updater,
            }
        }

        pub fn with_session(
            session: u64,
            state_updater: overwatch::services::state::StateUpdater<
                Option<RecoveryServiceState<BackendSettings, BroadcastSettings>>,
            >,
        ) -> Self {
            Self::new(session, 0, HashSet::new(), state_updater)
        }

        pub const fn start_updating(self) -> StateUpdater<BackendSettings, BroadcastSettings> {
            StateUpdater::new(self)
        }

        pub const fn last_seen_session(&self) -> u64 {
            self.last_seen_session
        }

        pub(super) const fn spend_quota(&mut self, quota: u64) {
            self.spent_core_quota = self
                .spent_core_quota
                .checked_add(quota)
                .expect("Spent core quota addition overflow.");
        }

        pub const fn spent_core_quota(&self) -> u64 {
            self.spent_core_quota
        }

        #[expect(
            clippy::type_complexity,
            reason = "Just a tuple over the struct's fields."
        )]
        pub fn into_components(
            self,
        ) -> (
            u64,
            u64,
            HashSet<ProcessedMessage<BroadcastSettings>>,
            overwatch::services::state::StateUpdater<
                Option<RecoveryServiceState<BackendSettings, BroadcastSettings>>,
            >,
        ) {
            (
                self.last_seen_session,
                self.spent_core_quota,
                self.unsent_processed_messages,
                self.state_updater,
            )
        }

        pub(super) const fn state_updater(
            &self,
        ) -> &overwatch::services::state::StateUpdater<
            Option<RecoveryServiceState<BackendSettings, BroadcastSettings>>,
        > {
            &self.state_updater
        }
    }

    impl<BackendSettings, BroadcastSettings> ServiceState<BackendSettings, BroadcastSettings>
    where
        BroadcastSettings: Eq + Hash,
    {
        pub(super) fn add_unsent_message(
            &mut self,
            message: ProcessedMessage<BroadcastSettings>,
        ) -> Result<(), ()> {
            if self.unsent_processed_messages.insert(message) {
                Ok(())
            } else {
                Err(())
            }
        }

        pub(super) fn remove_sent_message(
            &mut self,
            message: &ProcessedMessage<BroadcastSettings>,
        ) -> Result<(), ()> {
            if self.unsent_processed_messages.remove(message) {
                Ok(())
            } else {
                Err(())
            }
        }

        pub const fn unsent_processed_messages(
            &self,
        ) -> &HashSet<ProcessedMessage<BroadcastSettings>> {
            &self.unsent_processed_messages
        }

        pub(super) fn take_unsent_processed_messages(
            &mut self,
        ) -> impl Iterator<Item = ProcessedMessage<BroadcastSettings>> {
            self.unsent_processed_messages.drain()
        }
    }
}

mod state_updater {
    use core::hash::Hash;

    use crate::{core::state::service::ServiceState, message::ProcessedMessage};

    pub struct StateUpdater<BackendSettings, BroadcastSettings> {
        inner: ServiceState<BackendSettings, BroadcastSettings>,
        changed: bool,
    }

    impl<BackendSettings, BroadcastSettings> StateUpdater<BackendSettings, BroadcastSettings> {
        pub(super) const fn new(inner: ServiceState<BackendSettings, BroadcastSettings>) -> Self {
            Self {
                inner,
                changed: false,
            }
        }

        pub fn into_inner(self) -> ServiceState<BackendSettings, BroadcastSettings> {
            self.inner
        }

        pub const fn consume_core_quota(&mut self, amount: u64) {
            self.changed = true;
            self.inner.spend_quota(amount);
        }

        pub fn consume_without_committing(
            self,
        ) -> ServiceState<BackendSettings, BroadcastSettings> {
            self.inner
        }
    }

    impl<BackendSettings, BroadcastSettings> StateUpdater<BackendSettings, BroadcastSettings>
    where
        BackendSettings: Clone,
        BroadcastSettings: Clone,
    {
        pub fn commit_changes(self) -> ServiceState<BackendSettings, BroadcastSettings> {
            if self.changed {
                self.inner
                    .state_updater()
                    .update(Some(self.inner.clone().into()));
            }
            self.inner
        }
    }

    impl<BackendSettings, BroadcastSettings> StateUpdater<BackendSettings, BroadcastSettings>
    where
        BroadcastSettings: Eq + Hash,
    {
        pub fn add_unsent_message(
            &mut self,
            message: ProcessedMessage<BroadcastSettings>,
        ) -> Result<(), ()> {
            self.changed = true;
            self.inner.add_unsent_message(message)
        }

        pub fn remove_sent_message(
            &mut self,
            message: &ProcessedMessage<BroadcastSettings>,
        ) -> Result<(), ()> {
            self.changed = true;
            self.inner.remove_sent_message(message)
        }

        pub fn take_unsent_processed_messages(
            &mut self,
        ) -> impl Iterator<Item = ProcessedMessage<BroadcastSettings>> {
            self.changed = true;
            self.inner.take_unsent_processed_messages()
        }
    }
}

pub use self::recovery_state::RecoveryServiceState;
mod recovery_state {
    use core::{convert::Infallible, marker::PhantomData};

    use serde::{Deserialize, Serialize};

    use crate::core::{
        settings::BlendConfig,
        state::{ServiceState, serde::SerializableServiceState},
    };

    #[derive(Clone, Serialize, Deserialize)]
    pub struct RecoveryServiceState<BackendSettings, BroadcastSettings> {
        #[serde(bound(
            deserialize = "BroadcastSettings: Deserialize<'de> + Eq + core::hash::Hash"
        ))]
        pub service_state: Option<SerializableServiceState<BroadcastSettings>>,
        _phantom: PhantomData<BackendSettings>,
    }

    impl<BackendSettings, BroadcastSettings> From<ServiceState<BackendSettings, BroadcastSettings>>
        for RecoveryServiceState<BackendSettings, BroadcastSettings>
    {
        fn from(value: ServiceState<BackendSettings, BroadcastSettings>) -> Self {
            Self {
                _phantom: PhantomData,
                service_state: Some(value.into()),
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
}
