use std::{
    fmt::{Debug, Display},
    time::SystemTime,
};

use async_trait::async_trait;
use lb_libp2p::PeerId;
use lb_services_utils::overwatch::recovery::RecoveryOperator;
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceCore, ServiceData},
};
use tokio::sync::broadcast;

use crate::{
    BanningConfig,
    ban_store::{BanMutation, BanStore, Clock, SystemClock},
    recovery::{BanningRecoveryBackend, BanningRecoveryState},
    types::{BanEvent, BanRecord, BanScope, BanningRequest, Violation},
};

const EVENT_BUFFER_SIZE: usize = 256;

/// Mutable ban state retained by [`BanningService`].
///
/// Runtime event plumbing intentionally lives on the service itself so clones
/// retained by the state machinery cannot keep subscriptions alive after the
/// service stops.
#[derive(Debug, Clone)]
pub struct BanningState<C = SystemClock> {
    pub(crate) store: BanStore<C>,
}

impl BanningState<SystemClock> {
    fn restore(config: &BanningConfig, recovery_state: BanningRecoveryState) -> Self {
        let mut store = BanStore::from_config(config);
        store.restore_dynamic(recovery_state.runtime_records(config, SystemTime::now()));
        Self::with_store(store)
    }
}

struct StateTransition<T> {
    response: T,
    events: Vec<BanEvent>,
    recovery_changed: bool,
}

impl<C: Clock> BanningState<C> {
    pub(crate) const fn with_store(store: BanStore<C>) -> Self {
        Self { store }
    }

    fn expire(&mut self, events: &mut Vec<BanEvent>) -> bool {
        let expired = self.store.expire();
        let changed = !expired.is_empty();
        events.extend(expired.into_iter().map(|record| BanEvent::Unbanned {
            record,
            expired: true,
        }));
        changed
    }

    fn expire_transition(&mut self) -> StateTransition<()> {
        let mut events = Vec::new();
        let recovery_changed = self.expire(&mut events);
        StateTransition {
            response: (),
            events,
            recovery_changed,
        }
    }

    fn report(&mut self, violation: Violation) -> StateTransition<Option<BanRecord>> {
        let mut events = Vec::new();
        let mut recovery_changed = self.expire(&mut events);
        let mutation = self.store.report(violation);
        if let Some(mutation) = &mutation
            && !matches!(mutation, BanMutation::Unchanged(_))
        {
            events.push(BanEvent::Banned(mutation.record().clone()));
            recovery_changed = true;
        }
        StateTransition {
            response: mutation.map(|mutation| mutation.record().clone()),
            events,
            recovery_changed,
        }
    }

    fn replace(&mut self, violation: Violation) -> StateTransition<Option<BanRecord>> {
        let mut events = Vec::new();
        let mut recovery_changed = self.expire(&mut events);
        let mutation = self.store.replace(violation);
        if let Some(mutation) = &mutation
            && !matches!(mutation, BanMutation::Unchanged(_))
        {
            events.push(BanEvent::Replaced(mutation.record().clone()));
            recovery_changed = true;
        }
        StateTransition {
            response: mutation.map(|mutation| mutation.record().clone()),
            events,
            recovery_changed,
        }
    }

    fn active(&mut self) -> StateTransition<Vec<BanRecord>> {
        let mut events = Vec::new();
        let recovery_changed = self.expire(&mut events);
        StateTransition {
            response: self.store.active(),
            events,
            recovery_changed,
        }
    }

    fn unban(&mut self, peer_id: PeerId, scope: Option<&BanScope>) -> StateTransition<bool> {
        let mut events = Vec::new();
        let mut recovery_changed = self.expire(&mut events);
        let removed = self.store.unban(peer_id, scope);
        let was_unbanned = !removed.is_empty();
        if was_unbanned {
            recovery_changed = true;
        }
        events.extend(removed.into_iter().map(|record| BanEvent::Unbanned {
            record,
            expired: false,
        }));
        StateTransition {
            response: was_unbanned,
            events,
            recovery_changed,
        }
    }

    fn recovery_state(&self) -> BanningRecoveryState {
        BanningRecoveryState::from_store(&self.store)
    }
}

/// Central in-memory peer banning service.
pub struct BanningService<RuntimeServiceId> {
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    state: BanningState,
    events: broadcast::Sender<BanEvent>,
}

impl<RuntimeServiceId> ServiceData for BanningService<RuntimeServiceId> {
    type Settings = BanningConfig;
    type State = BanningRecoveryState;
    type StateOperator = RecoveryOperator<BanningRecoveryBackend<RuntimeServiceId>>;
    type Message = BanningRequest;
}

#[async_trait]
impl<RuntimeServiceId> ServiceCore<RuntimeServiceId> for BanningService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<lb_storage_service::StorageService<RuntimeServiceId>>
        + Clone
        + Display
        + Send
        + Sync
        + 'static
        + Debug,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        let config = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        let events = broadcast::channel(EVENT_BUFFER_SIZE).0;
        Ok(Self {
            service_resources_handle,
            state: BanningState::restore(&config, initial_state),
            events,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        self.service_resources_handle.status_updater.notify_ready();
        let mut expiry = tokio::time::interval(self.state.store.expiry_check_interval());
        expiry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = expiry.tick() => {
                    let transition = self.state.expire_transition();
                    self.commit(transition);
                }
                message = self.service_resources_handle.inbound_relay.recv() => {
                    let Some(message) = message else { break; };
                    self.handle(message);
                }
            }
        }

        Ok(())
    }
}

impl<RuntimeServiceId> BanningService<RuntimeServiceId> {
    fn publish(sender: &broadcast::Sender<BanEvent>, events: impl IntoIterator<Item = BanEvent>) {
        for event in events {
            let _unused = sender.send(event);
        }
    }

    fn commit<T>(&self, transition: StateTransition<T>) -> T {
        if transition.recovery_changed {
            // Queue the full checkpoint before publishing the state transition
            // or completing its request. Disk persistence remains asynchronous.
            self.service_resources_handle
                .state_updater
                .update(Some(self.state.recovery_state()));
        }
        Self::publish(&self.events, transition.events);
        transition.response
    }

    fn handle(&mut self, message: BanningRequest) {
        match message {
            BanningRequest::BanPeer { violation, reply } => {
                let transition = self.state.report(violation);
                let response = self.commit(transition);
                let _unused = reply.send(response);
            }
            BanningRequest::ReplaceBan { violation, reply } => {
                let transition = self.state.replace(violation);
                let response = self.commit(transition);
                let _unused = reply.send(response);
            }
            BanningRequest::ListActive { reply } => {
                let transition = self.state.active();
                let response = self.commit(transition);
                let _unused = reply.send(response);
            }
            BanningRequest::UnbanPeer {
                peer_id,
                scope,
                reply,
            } => {
                let transition = self.state.unban(peer_id, scope.as_ref());
                let response = self.commit(transition);
                let _unused = reply.send(response);
            }
            BanningRequest::Subscribe { reply } => {
                let _unused = reply.send(self.events.subscribe());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use super::*;
    use crate::{BanSource, OffenseKind, Subsystem};

    #[derive(Clone)]
    struct ManualClock(Arc<Mutex<SystemTime>>);

    impl ManualClock {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(SystemTime::UNIX_EPOCH)))
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> SystemTime {
            *self.0.lock().expect("clock lock")
        }
    }

    fn publish_transition<T>(
        sender: &broadcast::Sender<BanEvent>,
        transition: StateTransition<T>,
    ) -> T {
        BanningService::<()>::publish(sender, transition.events);
        transition.response
    }

    #[test]
    fn events_retain_scope_for_report_and_expiry() {
        let config = BanningConfig::default();
        let clock = ManualClock::new();
        let mut state = BanningState::with_store(BanStore::with_clock(&config, clock.clone()));
        let event_sender = broadcast::channel(EVENT_BUFFER_SIZE).0;
        let mut events = event_sender.subscribe();
        let peer_id = PeerId::random();
        let scope = BanScope::Service(Subsystem::ChainSync);
        let source = BanSource::Service(Subsystem::Other("validator".to_owned()));

        let transition = state.report(Violation::new(
            peer_id,
            source.clone(),
            scope.clone(),
            OffenseKind::Other,
            Duration::from_secs(5),
            Some("bad chain response".to_owned()),
        ));
        assert!(transition.recovery_changed);
        let _unused = publish_transition(&event_sender, transition);
        let BanEvent::Banned(record) = events.try_recv().expect("ban event") else {
            panic!("expected ban event");
        };
        assert_eq!(record.source, source);
        assert_eq!(record.scope, scope);
        assert_eq!(record.context.as_deref(), Some("bad chain response"));

        *clock.0.lock().expect("clock lock") = record.expires_at.expect("expiry");
        let expiry = state.expire_transition();
        assert!(expiry.recovery_changed);
        publish_transition(&event_sender, expiry);
        let BanEvent::Unbanned {
            record: expired_record,
            expired,
        } = events.try_recv().expect("expiry event")
        else {
            panic!("expected expiry event");
        };
        assert!(expired);
        assert_eq!(expired_record.peer_id, peer_id);
        assert_eq!(expired_record.source, source);
        assert_eq!(expired_record.scope, scope);
        assert_eq!(
            expired_record.context.as_deref(),
            Some("bad chain response")
        );
    }

    #[test]
    fn replacement_and_explicit_unban_events_retain_the_complete_record() {
        let config = BanningConfig::default();
        let clock = ManualClock::new();
        let mut state = BanningState::with_store(BanStore::with_clock(&config, clock));
        let event_sender: broadcast::Sender<BanEvent> = broadcast::channel(EVENT_BUFFER_SIZE).0;
        let mut events = event_sender.subscribe();
        let peer_id = PeerId::random();
        let scope = BanScope::Service(Subsystem::ChainSync);
        let source = BanSource::Service(Subsystem::Other("operator".to_owned()));

        let initial = state.report(Violation::new(
            peer_id,
            source.clone(),
            scope.clone(),
            OffenseKind::InvalidSig,
            Duration::from_secs(10),
            Some("initial".to_owned()),
        ));
        let _unused = publish_transition(&event_sender, initial);
        let _unused = events.try_recv().expect("initial event");

        let replacement = state.replace(Violation::new(
            peer_id,
            source.clone(),
            scope.clone(),
            OffenseKind::SpamMsg,
            Duration::from_secs(5),
            Some("replacement".to_owned()),
        ));
        let _unused = publish_transition(&event_sender, replacement);
        let BanEvent::Replaced(replaced) = events.try_recv().expect("replacement event") else {
            panic!("expected replacement event");
        };
        assert_eq!(replaced.source, source);
        assert_eq!(replaced.scope, scope);
        assert_eq!(replaced.offense, OffenseKind::SpamMsg);
        assert_eq!(replaced.context.as_deref(), Some("replacement"));

        let unban = state.unban(peer_id, Some(&scope));
        assert!(unban.recovery_changed);
        assert!(publish_transition(&event_sender, unban));
        let BanEvent::Unbanned {
            record: unbanned,
            expired,
        } = events.try_recv().expect("unban event")
        else {
            panic!("expected unban event");
        };
        assert!(!expired);
        assert_eq!(unbanned.source, source);
        assert_eq!(unbanned.scope, scope);
        assert_eq!(unbanned.offense, OffenseKind::SpamMsg);
        assert_eq!(unbanned.context.as_deref(), Some("replacement"));
    }

    #[test]
    fn failed_replace_does_not_create_a_record_or_emit_an_event() {
        let blacklisted_peer = PeerId::random();
        let config = BanningConfig {
            blacklist: vec![blacklisted_peer],
            ..Default::default()
        };
        let mut state = BanningState::with_store(BanStore::with_clock(&config, ManualClock::new()));
        let event_sender: broadcast::Sender<BanEvent> = broadcast::channel(EVENT_BUFFER_SIZE).0;
        let mut events = event_sender.subscribe();
        let missing_peer = PeerId::random();
        let chain_scope = BanScope::Service(Subsystem::ChainSync);

        let missing = state.replace(Violation::new(
            missing_peer,
            BanSource::Service(Subsystem::ChainSync),
            chain_scope,
            OffenseKind::ProtocolViolation,
            Duration::from_secs(5),
            None,
        ));
        assert!(!missing.recovery_changed);
        assert!(publish_transition(&event_sender, missing).is_none());
        let records = state.store.active();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].peer_id, blacklisted_peer);
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        let blacklisted = state.replace(Violation::new(
            blacklisted_peer,
            BanSource::Service(Subsystem::ChainSync),
            BanScope::Global,
            OffenseKind::ProtocolViolation,
            Duration::from_secs(5),
            None,
        ));
        assert!(!blacklisted.recovery_changed);
        assert!(publish_transition(&event_sender, blacklisted).is_none());
        assert_eq!(state.store.active().len(), 1);
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn recovered_state_is_installed_with_current_configuration_before_service_run() {
        let configured_peer = PeerId::random();
        let dynamic_peer = PeerId::random();
        let config = BanningConfig {
            blacklist: vec![configured_peer],
            ..BanningConfig::default()
        };
        let now = SystemTime::now();
        let mut checkpoint_store = BanStore::from_config(&config);
        checkpoint_store.restore_dynamic([BanRecord {
            peer_id: dynamic_peer,
            source: BanSource::Service(Subsystem::ChainSync),
            scope: BanScope::Service(Subsystem::ChainSync),
            offense: OffenseKind::ProtocolViolation,
            context: Some("recovered".to_owned()),
            reported_at: now,
            expires_at: Some(now + Duration::from_secs(60)),
        }]);

        let state =
            BanningState::restore(&config, BanningRecoveryState::from_store(&checkpoint_store));
        let active = state.store.active();

        assert!(
            active
                .iter()
                .any(|record| record.peer_id == configured_peer)
        );
        assert!(active.iter().any(|record| record.peer_id == dynamic_peer));
        assert_eq!(
            active
                .iter()
                .find(|record| record.peer_id == dynamic_peer)
                .and_then(|record| record.expires_at),
            Some(now + Duration::from_secs(60))
        );
    }
}
