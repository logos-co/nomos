use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, SystemTime},
};

use lb_libp2p::PeerId;
use lb_utils::tokio::task::{CancellableHandle, spawn};
use tokio::{
    sync::{broadcast, watch},
    time::Instant as TokioInstant,
};

use crate::{
    BanEvent, BanRecord, BanScope, BanSource, BanningApiError, BanningServiceApi,
    ConfiguredBanPolicy,
};

const LOG_TARGET: &str = lb_log_targets::ROOT;
const DYNAMIC_AUTHORITY_LEASE: Duration = Duration::from_secs(10);
const DYNAMIC_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const INITIAL_RETRY_BACKOFF: Duration = Duration::from_secs(1);
const MAX_RETRY_BACKOFF: Duration = Duration::from_secs(30);
const RELAY_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
pub struct LocalBanViewTiming {
    authority_lease: Duration,
    refresh_interval: Duration,
}

impl LocalBanViewTiming {
    const PRODUCTION: Self = Self {
        authority_lease: DYNAMIC_AUTHORITY_LEASE,
        refresh_interval: DYNAMIC_REFRESH_INTERVAL,
    };

    #[cfg(test)]
    #[must_use]
    pub const fn for_test(authority_lease: Duration, refresh_interval: Duration) -> Self {
        Self {
            authority_lease,
            refresh_interval,
        }
    }
}

/// A local, fail-open view of bans.
///
/// The view always enforces the configured blacklist. Dynamic records are
/// enforced only while the view has a valid lease from the central service.
/// Consumers perform synchronous reads; all interaction with the central
/// service happens in the single background synchronizer.
#[derive(Clone)]
pub struct LocalBanView {
    inner: Arc<LocalBanViewInner>,
}

struct LocalBanViewInner {
    configured_policy: ConfiguredBanPolicy,
    state: Arc<RwLock<LocalBanViewState>>,
    synchronizer: Mutex<Option<CancellableHandle<()>>>,
    timing: LocalBanViewTiming,
}

#[derive(Debug)]
struct LocalBanViewState {
    dynamic_authority_valid_until: Option<TokioInstant>,
    dynamic_records: HashMap<(PeerId, BanScope), BanRecord>,
    policy_changed: watch::Sender<()>,
}

impl Default for LocalBanViewState {
    fn default() -> Self {
        let (policy_changed, _) = watch::channel(());
        Self {
            dynamic_authority_valid_until: None,
            dynamic_records: HashMap::new(),
            policy_changed,
        }
    }
}

impl LocalBanView {
    /// Creates a view and, when an API is supplied, starts its one
    /// cancellation-bound synchronizer.
    #[must_use]
    pub fn new<RuntimeServiceId>(
        banning_service: Option<BanningServiceApi<RuntimeServiceId>>,
        configured_policy: ConfiguredBanPolicy,
    ) -> Self
    where
        RuntimeServiceId: Send + Sync + 'static,
    {
        Self::construct_with_timing(
            banning_service,
            configured_policy,
            LocalBanViewTiming::PRODUCTION,
        )
    }

    #[cfg(test)]
    #[must_use]
    pub fn new_with_timing<RuntimeServiceId>(
        banning_service: Option<BanningServiceApi<RuntimeServiceId>>,
        configured_policy: ConfiguredBanPolicy,
        timing: LocalBanViewTiming,
    ) -> Self
    where
        RuntimeServiceId: Send + Sync + 'static,
    {
        Self::construct_with_timing(banning_service, configured_policy, timing)
    }

    fn construct_with_timing<RuntimeServiceId>(
        banning_service: Option<BanningServiceApi<RuntimeServiceId>>,
        configured_policy: ConfiguredBanPolicy,
        timing: LocalBanViewTiming,
    ) -> Self
    where
        RuntimeServiceId: Send + Sync + 'static,
    {
        let view = Self {
            inner: Arc::new(LocalBanViewInner {
                configured_policy,
                state: Arc::new(RwLock::new(LocalBanViewState::default())),
                synchronizer: Mutex::new(None),
                timing,
            }),
        };

        if let Some(banning_service) = banning_service {
            view.start_synchronizer(banning_service);
        }

        view
    }

    /// Starts synchronization if this view does not already own a
    /// synchronizer. This is useful when a relay is obtained after the view
    /// has been constructed; repeated calls never create another task.
    pub fn start_synchronizer<RuntimeServiceId>(
        &self,
        banning_service: BanningServiceApi<RuntimeServiceId>,
    ) where
        RuntimeServiceId: Send + Sync + 'static,
    {
        let state = Arc::clone(&self.inner.state);
        let timing = self.inner.timing;
        self.start_task(async move {
            Self::run_synchronizer(banning_service, state, timing).await;
        });
    }

    /// Starts one long-lived task that retries initial relay acquisition with
    /// bounded exponential backoff, then hands the acquired API to the normal
    /// local-view synchronizer. Each acquisition attempt is time-bounded.
    pub fn start_synchronizer_with_acquisition<RuntimeServiceId, Acquire, AcquireFuture>(
        &self,
        mut acquire: Acquire,
    ) where
        RuntimeServiceId: Send + Sync + 'static,
        Acquire: FnMut() -> AcquireFuture + Send + 'static,
        AcquireFuture:
            Future<Output = Result<BanningServiceApi<RuntimeServiceId>, String>> + Send + 'static,
    {
        let state = Arc::clone(&self.inner.state);
        let timing = self.inner.timing;
        self.start_task(async move {
            let mut retry_backoff = INITIAL_RETRY_BACKOFF;
            let mut reported_unavailable = false;

            loop {
                match tokio::time::timeout(RELAY_ACQUIRE_TIMEOUT, acquire()).await {
                    Ok(Ok(banning_service)) => {
                        if reported_unavailable {
                            tracing::info!(target: LOG_TARGET, "banning service relay acquired; local view synchronizing");
                        }
                        Self::run_synchronizer(banning_service, state, timing).await;
                        return;
                    }
                    Ok(Err(error)) => {
                        if !reported_unavailable {
                            tracing::warn!(target: LOG_TARGET, %error, "banning service relay unavailable; local view remains fail-open");
                            reported_unavailable = true;
                        }
                    }
                    Err(_) => {
                        if !reported_unavailable {
                            tracing::warn!(target: LOG_TARGET, "banning service relay acquisition timed out; local view remains fail-open");
                            reported_unavailable = true;
                        }
                    }
                }

                tokio::time::sleep(retry_backoff).await;
                retry_backoff = Self::next_retry_backoff(retry_backoff, MAX_RETRY_BACKOFF);
            }
        });
    }

    fn start_task(&self, task: impl Future<Output = ()> + Send + 'static) {
        let Ok(mut synchronizer) = self.inner.synchronizer.lock() else {
            tracing::error!(target: LOG_TARGET, "failed to lock local banning synchronizer state");
            return;
        };

        if synchronizer.is_some() {
            return;
        }

        *synchronizer = Some(CancellableHandle::new(spawn(
            "logos/banning/local-view-sync",
            task,
        )));
    }

    /// Subscribes to coalescing policy-change notifications for connection
    /// reconciliation. The notification carries no state; consumers must
    /// re-evaluate the current view after waking.
    #[must_use]
    pub fn subscribe_policy_changes(&self) -> watch::Receiver<()> {
        self.inner.state.read().map_or_else(
            |_| watch::channel(()).1,
            |state| state.policy_changed.subscribe(),
        )
    }

    /// Returns whether a peer is banned for the requested consumer scope.
    /// This method never waits for the central service.
    #[must_use]
    pub fn is_banned_for(&self, peer_id: PeerId, scope: &BanScope) -> bool {
        let now = SystemTime::now();
        let Ok(state) = self.inner.state.read() else {
            return self.inner.configured_policy.contains(&peer_id);
        };

        self.inner.configured_policy.contains(&peer_id)
            || (Self::authority_valid_at(&state, TokioInstant::now())
                && Self::has_effective_dynamic_ban(&state, peer_id, scope, now))
    }

    /// Removes banned peers from a candidate set before a consumer applies
    /// its own bounded selection policy.
    #[must_use]
    pub fn filter_for(&self, peers: &HashSet<PeerId>, scope: &BanScope) -> HashSet<PeerId> {
        let now = SystemTime::now();
        let Ok(state) = self.inner.state.read() else {
            return peers
                .iter()
                .copied()
                .filter(|peer_id| !self.inner.configured_policy.contains(peer_id))
                .collect();
        };

        let authority_valid = Self::authority_valid_at(&state, TokioInstant::now());
        peers
            .iter()
            .copied()
            .filter(|peer_id| {
                !self.inner.configured_policy.contains(peer_id)
                    && (!authority_valid
                        || !Self::has_effective_dynamic_ban(&state, *peer_id, scope, now))
            })
            .collect()
    }

    /// Installs an authoritative dynamic snapshot atomically.
    pub fn replace_dynamic_snapshot(&self, records: Vec<BanRecord>) {
        Self::install_snapshot_in(
            &self.inner.state,
            records,
            self.inner.timing.authority_lease,
        );
    }

    /// Invalidates dynamic authority while preserving configured policy.
    pub fn invalidate_dynamic_authority(&self) {
        Self::disable_in(&self.inner.state);
    }

    fn is_effective_at(record: &BanRecord, now: SystemTime) -> bool {
        record.expires_at.is_none_or(|expiry| now < expiry)
    }

    fn has_effective_dynamic_ban(
        state: &LocalBanViewState,
        peer_id: PeerId,
        scope: &BanScope,
        now: SystemTime,
    ) -> bool {
        state
            .dynamic_records
            .get(&(peer_id, BanScope::Global))
            .is_some_and(|record| Self::is_effective_at(record, now))
            || (scope != &BanScope::Global
                && state
                    .dynamic_records
                    .get(&(peer_id, scope.clone()))
                    .is_some_and(|record| Self::is_effective_at(record, now)))
    }

    fn authority_valid_at(state: &LocalBanViewState, now: TokioInstant) -> bool {
        state
            .dynamic_authority_valid_until
            .is_some_and(|valid_until| now < valid_until)
    }

    fn install_snapshot_in(
        state: &Arc<RwLock<LocalBanViewState>>,
        records: Vec<BanRecord>,
        authority_lease: Duration,
    ) {
        let Ok(mut state_guard) = state.write() else {
            return;
        };

        let wall_now = SystemTime::now();
        let monotonic_now = TokioInstant::now();
        let replacement = records
            .into_iter()
            .filter(|record| {
                record.source != BanSource::Configuration && Self::is_effective_at(record, wall_now)
            })
            .map(|record| ((record.peer_id, record.scope.clone()), record))
            .collect();
        let previously_effective =
            Self::effective_dynamic_records(&state_guard, wall_now, monotonic_now);
        let policy_changed = previously_effective != replacement;
        state_guard.dynamic_records = replacement;
        state_guard.dynamic_authority_valid_until = Some(monotonic_now + authority_lease);
        let notification = policy_changed.then(|| state_guard.policy_changed.clone());
        drop(state_guard);
        if let Some(notification) = notification {
            notification.send_modify(|()| {});
        }
    }

    fn disable_in(state: &Arc<RwLock<LocalBanViewState>>) {
        let Ok(mut state_guard) = state.write() else {
            return;
        };
        let policy_changed = Self::clear_dynamic_state(&mut state_guard);
        let notification = policy_changed.then(|| state_guard.policy_changed.clone());
        drop(state_guard);
        if let Some(notification) = notification {
            notification.send_modify(|()| {});
        }
    }

    fn apply_event_in(state: &Arc<RwLock<LocalBanViewState>>, event: BanEvent) {
        let Ok(mut state_guard) = state.write() else {
            return;
        };

        let monotonic_now = TokioInstant::now();
        if !Self::authority_valid_at(&state_guard, monotonic_now) {
            let policy_changed = Self::clear_dynamic_state(&mut state_guard);
            let notification = policy_changed.then(|| state_guard.policy_changed.clone());
            drop(state_guard);
            if let Some(notification) = notification {
                notification.send_modify(|()| {});
            }
            return;
        }

        let wall_now = SystemTime::now();
        let previously_effective =
            Self::effective_dynamic_records(&state_guard, wall_now, monotonic_now);

        match event {
            BanEvent::Banned(record) | BanEvent::Replaced(record) => {
                if record.source == BanSource::Configuration {
                    return;
                }
                let key = (record.peer_id, record.scope.clone());
                if Self::is_effective_at(&record, wall_now) {
                    state_guard.dynamic_records.insert(key, record);
                } else {
                    state_guard.dynamic_records.remove(&key);
                }
            }
            BanEvent::Unbanned { record, .. } => {
                if record.source != BanSource::Configuration {
                    state_guard
                        .dynamic_records
                        .remove(&(record.peer_id, record.scope));
                }
            }
        }
        let policy_changed = previously_effective
            != Self::effective_dynamic_records(&state_guard, wall_now, monotonic_now);
        let notification = policy_changed.then(|| state_guard.policy_changed.clone());
        drop(state_guard);
        if let Some(notification) = notification {
            notification.send_modify(|()| {});
        }
    }

    fn clear_dynamic_state(state: &mut LocalBanViewState) -> bool {
        let has_unexpired_records = state
            .dynamic_records
            .values()
            .any(|record| Self::is_effective_at(record, SystemTime::now()));
        state.dynamic_authority_valid_until = None;
        state.dynamic_records.clear();
        has_unexpired_records
    }

    fn effective_dynamic_records(
        state: &LocalBanViewState,
        wall_now: SystemTime,
        monotonic_now: TokioInstant,
    ) -> HashMap<(PeerId, BanScope), BanRecord> {
        if !Self::authority_valid_at(state, monotonic_now) {
            return HashMap::new();
        }

        state
            .dynamic_records
            .iter()
            .filter(|(_, record)| Self::is_effective_at(record, wall_now))
            .map(|(key, record)| (key.clone(), record.clone()))
            .collect()
    }

    async fn synchronize<RuntimeServiceId>(
        banning_service: &BanningServiceApi<RuntimeServiceId>,
        state: &Arc<RwLock<LocalBanViewState>>,
        timing: LocalBanViewTiming,
    ) -> Result<broadcast::Receiver<BanEvent>, BanningApiError>
    where
        RuntimeServiceId: Send + Sync,
    {
        let receiver = banning_service.subscribe().await.inspect_err(|_| {
            Self::disable_in(state);
        })?;
        Self::refresh_dynamic(banning_service, state, timing)
            .await
            .inspect_err(|_| {
                Self::disable_in(state);
            })?;
        Ok(receiver)
    }

    async fn refresh_dynamic<RuntimeServiceId>(
        banning_service: &BanningServiceApi<RuntimeServiceId>,
        state: &Arc<RwLock<LocalBanViewState>>,
        timing: LocalBanViewTiming,
    ) -> Result<(), BanningApiError>
    where
        RuntimeServiceId: Send + Sync,
    {
        match banning_service.list_active().await {
            Ok(records) => {
                Self::install_snapshot_in(state, records, timing.authority_lease);
                Ok(())
            }
            Err(error) => {
                Self::disable_in(state);
                Err(error)
            }
        }
    }

    async fn run_synchronizer<RuntimeServiceId>(
        banning_service: BanningServiceApi<RuntimeServiceId>,
        state: Arc<RwLock<LocalBanViewState>>,
        timing: LocalBanViewTiming,
    ) where
        RuntimeServiceId: Send + Sync + 'static,
    {
        Self::run_synchronizer_with_backoff(
            banning_service,
            state,
            INITIAL_RETRY_BACKOFF,
            MAX_RETRY_BACKOFF,
            timing,
        )
        .await;
    }

    #[cfg(test)]
    async fn run_synchronizer_for_test<RuntimeServiceId>(
        banning_service: BanningServiceApi<RuntimeServiceId>,
        state: Arc<RwLock<LocalBanViewState>>,
        initial_retry_backoff: Duration,
        max_retry_backoff: Duration,
    ) where
        RuntimeServiceId: Send + Sync + 'static,
    {
        Self::run_synchronizer_with_backoff(
            banning_service,
            state,
            initial_retry_backoff,
            max_retry_backoff,
            LocalBanViewTiming::PRODUCTION,
        )
        .await;
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "The retry state machine deliberately keeps authority transitions together."
    )]
    async fn run_synchronizer_with_backoff<RuntimeServiceId>(
        banning_service: BanningServiceApi<RuntimeServiceId>,
        state: Arc<RwLock<LocalBanViewState>>,
        initial_retry_backoff: Duration,
        max_retry_backoff: Duration,
        timing: LocalBanViewTiming,
    ) where
        RuntimeServiceId: Send + Sync + 'static,
    {
        let mut retry_backoff = initial_retry_backoff;
        let mut unavailable = false;

        loop {
            match Self::synchronize(&banning_service, &state, timing).await {
                Ok(mut receiver) => {
                    tracing::info!(target: LOG_TARGET, "dynamic banning authority established");
                    retry_backoff = initial_retry_backoff;
                    unavailable = false;
                    Self::monitor_subscription_with_timing(
                        &banning_service,
                        &mut receiver,
                        &state,
                        timing,
                    )
                    .await;
                }
                Err(error) => {
                    Self::disable_in(&state);
                    if unavailable {
                        tracing::debug!(
                            target: LOG_TARGET,
                            %error,
                            "dynamic banning authority refresh failed; enforcement remains disabled"
                        );
                    } else {
                        tracing::warn!(
                            target: LOG_TARGET,
                            %error,
                            "dynamic banning authority unavailable; enforcement disabled"
                        );
                        unavailable = true;
                    }
                }
            }

            tokio::time::sleep(retry_backoff).await;
            retry_backoff = retry_backoff.saturating_mul(2).min(max_retry_backoff);
        }
    }

    fn next_retry_backoff(current: Duration, maximum: Duration) -> Duration {
        current.saturating_mul(2).min(maximum)
    }

    #[cfg(test)]
    async fn monitor_subscription<RuntimeServiceId>(
        banning_service: &BanningServiceApi<RuntimeServiceId>,
        receiver: &mut broadcast::Receiver<BanEvent>,
        state: &Arc<RwLock<LocalBanViewState>>,
    ) where
        RuntimeServiceId: Send + Sync,
    {
        Self::monitor_subscription_with_timing(
            banning_service,
            receiver,
            state,
            LocalBanViewTiming::PRODUCTION,
        )
        .await;
    }

    #[cfg(test)]
    async fn monitor_subscription_with_refresh_interval<RuntimeServiceId>(
        banning_service: &BanningServiceApi<RuntimeServiceId>,
        receiver: &mut broadcast::Receiver<BanEvent>,
        state: &Arc<RwLock<LocalBanViewState>>,
        refresh_interval: Duration,
    ) where
        RuntimeServiceId: Send + Sync,
    {
        Self::monitor_subscription_with_timing(
            banning_service,
            receiver,
            state,
            LocalBanViewTiming {
                refresh_interval,
                ..LocalBanViewTiming::PRODUCTION
            },
        )
        .await;
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "The subscription monitor owns the bounded lease and refresh transitions."
    )]
    async fn monitor_subscription_with_timing<RuntimeServiceId>(
        banning_service: &BanningServiceApi<RuntimeServiceId>,
        receiver: &mut broadcast::Receiver<BanEvent>,
        state: &Arc<RwLock<LocalBanViewState>>,
        timing: LocalBanViewTiming,
    ) where
        RuntimeServiceId: Send + Sync,
    {
        let deadline = Self::lease_deadline(state).unwrap_or_else(TokioInstant::now);
        let mut lease_expiry = Box::pin(tokio::time::sleep_until(deadline));
        let mut refresh = Box::pin(tokio::time::sleep(timing.refresh_interval));

        loop {
            if !Self::authority_valid(state) {
                Self::disable_in(state);
                tracing::warn!(target: LOG_TARGET, "dynamic banning authority lease expired");
                return;
            }

            tokio::select! {
                event = receiver.recv() => match event {
                    Ok(event) => Self::apply_event_in(state, event),
                    Err(broadcast::error::RecvError::Closed) => {
                        Self::disable_in(state);
                        tracing::warn!(target: LOG_TARGET, "banning subscription closed; dynamic enforcement disabled");
                        return;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        Self::disable_in(state);
                        tracing::warn!(target: LOG_TARGET, skipped, "banning subscription lagged; dynamic enforcement disabled");
                        return;
                    }
                },
                () = &mut lease_expiry => {
                    Self::disable_in(state);
                    tracing::warn!(target: LOG_TARGET, "dynamic banning authority lease expired");
                    return;
                }
                () = &mut refresh => {
                    match Self::refresh_dynamic(banning_service, state, timing).await {
                        Ok(()) => {
                            refresh.as_mut().reset(TokioInstant::now() + timing.refresh_interval);
                            if let Some(deadline) = Self::lease_deadline(state) {
                                lease_expiry.as_mut().reset(deadline);
                            }
                        }
                        Err(error) => {
                            Self::disable_in(state);
                            tracing::warn!(target: LOG_TARGET, %error, "authoritative banning refresh failed; dynamic enforcement disabled");
                            return;
                        }
                    }
                }
            }
        }
    }

    fn lease_deadline(state: &Arc<RwLock<LocalBanViewState>>) -> Option<TokioInstant> {
        state.read().ok()?.dynamic_authority_valid_until
    }

    fn authority_valid(state: &Arc<RwLock<LocalBanViewState>>) -> bool {
        state
            .read()
            .is_ok_and(|state| Self::authority_valid_at(&state, TokioInstant::now()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use overwatch::services::relay::OutboundRelay;
    use tokio::sync::{mpsc, oneshot};

    use super::*;
    use crate::{BanningConfig, BanningRequest, OffenseKind, Subsystem};

    #[test]
    fn production_timing_defaults_remain_unchanged() {
        let timing = LocalBanViewTiming::PRODUCTION;
        assert_eq!(timing.authority_lease, Duration::from_secs(10));
        assert_eq!(timing.refresh_interval, Duration::from_secs(5));
        assert_eq!(INITIAL_RETRY_BACKOFF, Duration::from_secs(1));
        assert_eq!(MAX_RETRY_BACKOFF, Duration::from_secs(30));
        assert_eq!(RELAY_ACQUIRE_TIMEOUT, Duration::from_secs(1));
    }

    fn record(peer_id: PeerId, scope: BanScope, source: BanSource) -> BanRecord {
        BanRecord {
            peer_id,
            source,
            scope,
            offense: OffenseKind::ProtocolViolation,
            context: Some("test".to_owned()),
            reported_at: SystemTime::UNIX_EPOCH,
            expires_at: Some(SystemTime::now() + Duration::from_secs(60)),
        }
    }

    #[test]
    fn scope_filtering_and_configured_policy_are_independent() {
        let chain_peer = PeerId::random();
        let blend_peer = PeerId::random();
        let global_peer = PeerId::random();
        let configured_peer = PeerId::random();
        let policy = ConfiguredBanPolicy::from_config(&BanningConfig {
            blacklist: vec![configured_peer],
            ..Default::default()
        });
        let view = LocalBanView::new::<()>(None, policy);
        view.replace_dynamic_snapshot(vec![
            record(
                chain_peer,
                BanScope::Service(Subsystem::ChainSync),
                BanSource::Service(Subsystem::ChainSync),
            ),
            record(
                blend_peer,
                BanScope::Service(Subsystem::Blend),
                BanSource::Service(Subsystem::Blend),
            ),
            record(
                global_peer,
                BanScope::Global,
                BanSource::Service(Subsystem::Other("policy".to_owned())),
            ),
        ]);

        let chain_scope = BanScope::Service(Subsystem::ChainSync);
        let blend_scope = BanScope::Service(Subsystem::Blend);
        assert!(view.is_banned_for(chain_peer, &chain_scope));
        assert!(!view.is_banned_for(blend_peer, &chain_scope));
        assert!(view.is_banned_for(global_peer, &chain_scope));
        assert!(view.is_banned_for(configured_peer, &blend_scope));
    }

    #[tokio::test(start_paused = true)]
    async fn lease_expiry_fails_open_but_configured_policy_remains() {
        let dynamic_peer = PeerId::random();
        let configured_peer = PeerId::random();
        let policy = ConfiguredBanPolicy::from_config(&BanningConfig {
            blacklist: vec![configured_peer],
            ..Default::default()
        });
        let view = LocalBanView::new::<()>(None, policy);
        let scope = BanScope::Service(Subsystem::ChainSync);
        view.replace_dynamic_snapshot(vec![record(
            dynamic_peer,
            scope.clone(),
            BanSource::Service(Subsystem::ChainSync),
        )]);
        let mut changes = view.subscribe_policy_changes();
        drop(changes.borrow_and_update());

        let (relay_sender, _requests) = mpsc::channel(1);
        let api = BanningServiceApi::<()>::new(OutboundRelay::new(relay_sender));
        let (event_sender, mut receiver) = broadcast::channel(1);
        let state = Arc::clone(&view.inner.state);
        let monitor = tokio::spawn(async move {
            LocalBanView::monitor_subscription_with_refresh_interval(
                &api,
                &mut receiver,
                &state,
                Duration::from_secs(60),
            )
            .await;
        });
        tokio::task::yield_now().await;

        assert!(view.is_banned_for(dynamic_peer, &scope));
        tokio::time::advance(DYNAMIC_AUTHORITY_LEASE + Duration::from_secs(1)).await;
        monitor.await.expect("lease monitor exits");
        assert!(!view.is_banned_for(dynamic_peer, &scope));
        assert!(view.is_banned_for(configured_peer, &scope));
        assert!(changes.has_changed().expect("lease expiry notified"));
        drop(event_sender);
    }

    #[tokio::test(start_paused = true)]
    async fn event_after_lease_expiry_clears_records_and_notifies_policy_change() {
        let peer = PeerId::random();
        let scope = BanScope::Service(Subsystem::ChainSync);
        let record = record(
            peer,
            scope.clone(),
            BanSource::Service(Subsystem::ChainSync),
        );
        let view = LocalBanView::new::<()>(None, ConfiguredBanPolicy::default());
        let mut changes = view.subscribe_policy_changes();
        view.replace_dynamic_snapshot(vec![record.clone()]);
        drop(changes.borrow_and_update());

        {
            let mut state = view.inner.state.write().expect("state");
            state.dynamic_authority_valid_until =
                Some(TokioInstant::now() - Duration::from_secs(1));
        };

        LocalBanView::apply_event_in(&view.inner.state, BanEvent::Banned(record));

        assert!(!view.is_banned_for(peer, &scope));
        assert!(
            changes
                .has_changed()
                .expect("stale event invalidation notified")
        );
        assert!(
            view.inner
                .state
                .read()
                .expect("state")
                .dynamic_records
                .is_empty()
        );
    }

    #[test]
    fn expired_finite_records_are_not_enforced_without_an_event() {
        let peer = PeerId::random();
        let mut record = record(
            peer,
            BanScope::Service(Subsystem::ChainSync),
            BanSource::Service(Subsystem::ChainSync),
        );
        record.expires_at = Some(SystemTime::UNIX_EPOCH);
        let view = LocalBanView::new::<()>(None, ConfiguredBanPolicy::default());
        view.replace_dynamic_snapshot(vec![record]);
        assert!(!view.is_banned_for(peer, &BanScope::Service(Subsystem::ChainSync)));
    }

    #[test]
    fn invalidation_does_not_clear_configured_policy() {
        let peer = PeerId::random();
        let policy = ConfiguredBanPolicy::from_config(&BanningConfig {
            blacklist: vec![peer],
            ..Default::default()
        });
        let view = LocalBanView::new::<()>(None, policy);
        view.invalidate_dynamic_authority();
        assert!(view.is_banned_for(peer, &BanScope::Global));
    }

    #[tokio::test(start_paused = true)]
    async fn identical_snapshots_renew_authority_without_policy_notifications() {
        let peer = PeerId::random();
        let scope = BanScope::Service(Subsystem::ChainSync);
        let original = record(
            peer,
            scope.clone(),
            BanSource::Service(Subsystem::ChainSync),
        );
        let view = LocalBanView::new::<()>(None, ConfiguredBanPolicy::default());
        let mut changes = view.subscribe_policy_changes();

        view.replace_dynamic_snapshot(vec![original.clone()]);
        assert!(changes.has_changed().expect("watch is open"));
        drop(changes.borrow_and_update());
        let first_lease = view
            .inner
            .state
            .read()
            .expect("state")
            .dynamic_authority_valid_until
            .expect("authority established");

        tokio::time::advance(Duration::from_secs(7)).await;
        view.replace_dynamic_snapshot(vec![original.clone()]);
        assert!(!changes.has_changed().expect("watch is open"));
        let renewed_lease = view
            .inner
            .state
            .read()
            .expect("state")
            .dynamic_authority_valid_until
            .expect("authority renewed");
        assert!(renewed_lease > first_lease);
        assert!(renewed_lease > TokioInstant::now());

        let mut changed_record = original.clone();
        changed_record.context = Some("authoritative replacement".to_owned());
        view.replace_dynamic_snapshot(vec![changed_record.clone()]);
        assert!(changes.has_changed().expect("snapshot change notified"));
        drop(changes.borrow_and_update());

        view.invalidate_dynamic_authority();
        assert!(changes.has_changed().expect("invalidation notified"));
        drop(changes.borrow_and_update());

        view.replace_dynamic_snapshot(vec![changed_record]);
        assert!(changes.has_changed().expect("recovery notified"));
        assert!(view.is_banned_for(peer, &scope));
    }

    #[tokio::test(start_paused = true)]
    async fn reordered_identical_snapshot_does_not_notify() {
        let first_peer = PeerId::random();
        let second_peer = PeerId::random();
        let first = record(
            first_peer,
            BanScope::Service(Subsystem::ChainSync),
            BanSource::Service(Subsystem::ChainSync),
        );
        let second = record(
            second_peer,
            BanScope::Global,
            BanSource::Service(Subsystem::Other("policy".to_owned())),
        );
        let view = LocalBanView::new::<()>(None, ConfiguredBanPolicy::default());
        let mut changes = view.subscribe_policy_changes();

        view.replace_dynamic_snapshot(vec![first.clone(), second.clone()]);
        drop(changes.borrow_and_update());
        view.replace_dynamic_snapshot(vec![second, first]);

        assert!(!changes.has_changed().expect("watch is open"));
    }

    #[tokio::test(start_paused = true)]
    async fn acquisition_attempts_are_timed_out_and_retried() {
        let configured_peer = PeerId::random();
        let view = LocalBanView::new::<()>(
            None,
            ConfiguredBanPolicy::from_config(&BanningConfig {
                blacklist: vec![configured_peer],
                ..Default::default()
            }),
        );
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_acquire = Arc::clone(&attempts);
        view.start_synchronizer_with_acquisition::<(), _, _>(move || {
            attempts_for_acquire.fetch_add(1, Ordering::SeqCst);
            async { std::future::pending::<Result<BanningServiceApi<()>, String>>().await }
        });

        tokio::task::yield_now().await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert!(view.is_banned_for(configured_peer, &BanScope::Global));

        tokio::time::advance(RELAY_ACQUIRE_TIMEOUT).await;
        tokio::task::yield_now().await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        tokio::time::advance(INITIAL_RETRY_BACKOFF).await;
        tokio::task::yield_now().await;
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn synchronizer_installs_snapshot_and_applies_unban_events() {
        let peer = PeerId::random();
        let active = record(
            peer,
            BanScope::Service(Subsystem::ChainSync),
            BanSource::Service(Subsystem::ChainSync),
        );
        let (sender, mut requests) = mpsc::channel(2);
        let (event_sender, _) = broadcast::channel(4);
        let event_sender_for_handler = event_sender.clone();
        let handler = tokio::spawn(async move {
            let BanningRequest::Subscribe { reply } = requests.recv().await.expect("subscribe")
            else {
                panic!("expected subscribe request");
            };
            reply
                .send(event_sender_for_handler.subscribe())
                .expect("subscription reply");
            let BanningRequest::ListActive { reply } = requests.recv().await.expect("snapshot")
            else {
                panic!("expected snapshot request");
            };
            reply.send(vec![active]).expect("snapshot reply");
        });
        let api = BanningServiceApi::<()>::new(OutboundRelay::new(sender));
        let view = LocalBanView::new(Some(api), ConfiguredBanPolicy::default());
        handler.await.expect("request handler");

        let scope = BanScope::Service(Subsystem::ChainSync);
        assert!(view.is_banned_for(peer, &scope));

        let state = Arc::clone(&view.inner.state);
        LocalBanView::apply_event_in(
            &state,
            BanEvent::Unbanned {
                record: record(
                    peer,
                    BanScope::Service(Subsystem::ChainSync),
                    BanSource::Service(Subsystem::ChainSync),
                ),
                expired: false,
            },
        );
        assert!(!view.is_banned_for(peer, &scope));
    }

    #[tokio::test]
    async fn subscription_close_and_lag_fail_open_without_clearing_configuration() {
        let dynamic_peer = PeerId::random();
        let configured_peer = PeerId::random();
        let policy = ConfiguredBanPolicy::from_config(&BanningConfig {
            blacklist: vec![configured_peer],
            ..Default::default()
        });
        let view = LocalBanView::new::<()>(None, policy);
        let scope = BanScope::Service(Subsystem::ChainSync);
        view.replace_dynamic_snapshot(vec![record(
            dynamic_peer,
            scope.clone(),
            BanSource::Service(Subsystem::ChainSync),
        )]);

        let (event_sender, receiver) = broadcast::channel(1);
        let (sender, _) = mpsc::channel(1);
        let api = BanningServiceApi::<()>::new(OutboundRelay::new(sender));
        let state = Arc::clone(&view.inner.state);
        let monitor = tokio::spawn(async move {
            let mut receiver = receiver;
            LocalBanView::monitor_subscription(&api, &mut receiver, &state).await;
        });
        drop(event_sender);
        monitor.await.expect("closed monitor");
        assert!(!view.is_banned_for(dynamic_peer, &scope));
        assert!(view.is_banned_for(configured_peer, &scope));

        view.replace_dynamic_snapshot(vec![record(
            dynamic_peer,
            scope.clone(),
            BanSource::Service(Subsystem::ChainSync),
        )]);
        let (event_sender, receiver) = broadcast::channel(1);
        let event = BanEvent::Banned(record(
            dynamic_peer,
            scope.clone(),
            BanSource::Service(Subsystem::ChainSync),
        ));
        event_sender.send(event.clone()).expect("first event");
        event_sender.send(event).expect("second event");
        let (sender, _) = mpsc::channel(1);
        let api = BanningServiceApi::<()>::new(OutboundRelay::new(sender));
        let state = Arc::clone(&view.inner.state);
        let monitor = tokio::spawn(async move {
            let mut receiver = receiver;
            LocalBanView::monitor_subscription(&api, &mut receiver, &state).await;
        });
        monitor.await.expect("lagged monitor");
        assert!(!view.is_banned_for(dynamic_peer, &scope));
        assert!(view.is_banned_for(configured_peer, &scope));
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_refresh_times_out_and_preserves_configured_policy() {
        let dynamic_peer = PeerId::random();
        let configured_peer = PeerId::random();
        let view = LocalBanView::new::<()>(
            None,
            ConfiguredBanPolicy::from_config(&BanningConfig {
                blacklist: vec![configured_peer],
                ..Default::default()
            }),
        );
        let scope = BanScope::Service(Subsystem::ChainSync);
        view.replace_dynamic_snapshot(vec![record(
            dynamic_peer,
            scope.clone(),
            BanSource::Service(Subsystem::ChainSync),
        )]);
        let (sender, mut requests) = mpsc::channel(1);
        let (seen_sender, seen_receiver) = oneshot::channel();
        let handler = tokio::spawn(async move {
            let BanningRequest::ListActive { reply } = requests.recv().await.expect("refresh")
            else {
                panic!("expected refresh request");
            };
            seen_sender.send(()).expect("refresh observed");
            let _reply = reply;
            std::future::pending::<()>().await;
        });
        let api = BanningServiceApi::<()>::new(OutboundRelay::new(sender));
        let state = Arc::clone(&view.inner.state);
        let refresh = tokio::spawn(async move {
            LocalBanView::refresh_dynamic(&api, &state, LocalBanViewTiming::PRODUCTION).await
        });
        seen_receiver.await.expect("refresh observed");
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(matches!(
            refresh.await.expect("refresh task"),
            Err(BanningApiError::Timeout)
        ));
        assert!(!view.is_banned_for(dynamic_peer, &scope));
        assert!(view.is_banned_for(configured_peer, &scope));
        handler.abort();
    }

    #[test]
    fn retry_backoff_is_exponential_and_capped() {
        let mut backoff = INITIAL_RETRY_BACKOFF;
        let mut observed = [Duration::ZERO; 8];
        for delay in &mut observed {
            *delay = backoff;
            backoff = LocalBanView::next_retry_backoff(backoff, MAX_RETRY_BACKOFF);
        }
        assert_eq!(
            observed,
            [
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
                Duration::from_secs(30),
                Duration::from_secs(30),
                Duration::from_secs(30),
            ]
        );
    }

    #[tokio::test]
    async fn synchronizer_recovers_after_more_failures_than_the_old_budget() {
        let peer = PeerId::random();
        let active = record(
            peer,
            BanScope::Service(Subsystem::ChainSync),
            BanSource::Service(Subsystem::ChainSync),
        );
        let (sender, mut requests) = mpsc::channel(8);
        let (event_sender, _) = broadcast::channel(8);
        let (recovered_sender, recovered_receiver) = oneshot::channel();
        let handler = tokio::spawn(async move {
            let mut failed = 0;
            let mut recovered_sender = Some(recovered_sender);
            while let Some(request) = requests.recv().await {
                match request {
                    BanningRequest::Subscribe { reply } if failed < 8 => {
                        failed += 1;
                        drop(reply);
                    }
                    BanningRequest::Subscribe { reply } => {
                        reply
                            .send(event_sender.subscribe())
                            .expect("subscription reply");
                    }
                    BanningRequest::ListActive { reply } => {
                        reply.send(vec![active.clone()]).expect("snapshot reply");
                        if let Some(sender) = recovered_sender.take() {
                            sender.send(()).expect("recovery remains alive");
                        }
                    }
                    _ => panic!("unexpected banning request"),
                }
            }
        });
        let api = BanningServiceApi::<()>::new(OutboundRelay::new(sender));
        let state = Arc::new(RwLock::new(LocalBanViewState::default()));
        let synchronizer = tokio::spawn(LocalBanView::run_synchronizer_for_test(
            api,
            Arc::clone(&state),
            Duration::ZERO,
            Duration::ZERO,
        ));
        recovered_receiver.await.expect("synchronizer recovers");
        let scope = BanScope::Service(Subsystem::ChainSync);
        assert!(
            state
                .read()
                .expect("state")
                .dynamic_records
                .values()
                .any(|record| record.peer_id == peer && record.applies_to(&scope))
        );
        synchronizer.abort();
        handler.abort();
    }

    #[tokio::test]
    async fn dropping_the_last_view_cancels_its_synchronizer() {
        let (sender, mut requests) = mpsc::channel(1);
        let api = BanningServiceApi::<()>::new(OutboundRelay::new(sender));
        let view = LocalBanView::new(Some(api), ConfiguredBanPolicy::default());
        let view_clone = view.clone();
        let _request = requests.recv().await.expect("subscribe request");
        drop(view);
        drop(view_clone);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), requests.recv())
                .await
                .expect("relay closes after view drop")
                .is_none()
        );
    }
}
