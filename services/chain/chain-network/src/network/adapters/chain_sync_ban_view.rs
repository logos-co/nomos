use std::collections::HashSet;

use lb_banning_service::{BanScope, LocalBanView, Subsystem};
use lb_network_service::backends::libp2p::PeerId;

/// `ChainSync`'s opt-in view of the central banning service.
///
/// The reusable local view owns synchronization and authority handling. This
/// wrapper keeps the `ChainSync` scope at its call sites and prevents those
/// consumers from accidentally querying another service's scope.
#[derive(Clone)]
pub struct ChainSyncBanView {
    local: LocalBanView,
}

impl ChainSyncBanView {
    pub(crate) const fn new(local: LocalBanView) -> Self {
        Self { local }
    }

    pub(crate) fn filter(&self, peers: &HashSet<PeerId>) -> HashSet<PeerId> {
        self.local.filter_for(peers, &Self::scope())
    }

    pub(crate) fn is_banned(&self, peer_id: PeerId) -> bool {
        self.local.is_banned_for(peer_id, &Self::scope())
    }

    #[cfg(test)]
    pub(crate) fn install_snapshot(&self, records: Vec<lb_banning_service::BanRecord>) {
        self.local.replace_dynamic_snapshot(records);
    }

    #[cfg(test)]
    pub(crate) fn disable(&self) {
        self.local.invalidate_dynamic_authority();
    }

    const fn scope() -> BanScope {
        BanScope::Service(Subsystem::ChainSync)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::{Duration, SystemTime},
    };

    use lb_banning_service::{
        BanRecord, BanSource, BanningConfig, BanningRequest, BanningServiceApi,
        ConfiguredBanPolicy, OffenseKind,
    };
    use overwatch::services::relay::OutboundRelay;
    use tokio::sync::{broadcast, mpsc, oneshot};

    use super::*;

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
    fn chain_sync_view_filters_only_applicable_scopes() {
        let chain_peer = PeerId::random();
        let blend_peer = PeerId::random();
        let global_peer = PeerId::random();
        let configured_peer = PeerId::random();
        let policy = ConfiguredBanPolicy::from_config(&BanningConfig {
            blacklist: vec![configured_peer],
            ..Default::default()
        });
        let view = ChainSyncBanView::new(LocalBanView::new::<()>(None, policy));
        let peers = HashSet::from([chain_peer, blend_peer, global_peer, configured_peer]);
        view.install_snapshot(vec![
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

        assert_eq!(view.filter(&peers), HashSet::from([blend_peer]));
        assert!(view.is_banned(chain_peer));
        assert!(!view.is_banned(blend_peer));
        assert!(view.is_banned(global_peer));
        assert!(view.is_banned(configured_peer));
    }

    #[test]
    fn dynamic_authority_loss_keeps_configured_blacklist_enforced() {
        let configured_peer = PeerId::random();
        let dynamic_peer = PeerId::random();
        let view = ChainSyncBanView::new(LocalBanView::new::<()>(
            None,
            ConfiguredBanPolicy::from_config(&BanningConfig {
                blacklist: vec![configured_peer],
                ..Default::default()
            }),
        ));
        view.install_snapshot(vec![record(
            dynamic_peer,
            BanScope::Service(Subsystem::ChainSync),
            BanSource::Service(Subsystem::ChainSync),
        )]);
        view.disable();

        assert!(!view.is_banned(dynamic_peer));
        assert!(view.is_banned(configured_peer));
    }

    #[tokio::test(start_paused = true)]
    async fn early_view_retries_unavailable_service_and_applies_dynamic_policy_after_recovery() {
        let configured_peer = PeerId::random();
        let dynamic_peer = PeerId::random();
        let healthy_peer = PeerId::random();
        let chain_scope = BanScope::Service(Subsystem::ChainSync);
        let configured_policy = ConfiguredBanPolicy::from_config(&BanningConfig {
            blacklist: vec![configured_peer],
            ..Default::default()
        });
        let active_record = record(
            dynamic_peer,
            chain_scope,
            BanSource::Service(Subsystem::ChainSync),
        );

        let (relay_sender, mut requests) = mpsc::channel(4);
        let api = BanningServiceApi::<()>::new(OutboundRelay::new(relay_sender));
        let (event_sender, _) = broadcast::channel(4);
        let (synchronized_sender, synchronized_receiver) = oneshot::channel();
        let handler = tokio::spawn(async move {
            let mut synchronized_sender = Some(synchronized_sender);
            while let Some(request) = requests.recv().await {
                match request {
                    BanningRequest::Subscribe { reply } => {
                        reply
                            .send(event_sender.subscribe())
                            .expect("subscription reply");
                    }
                    BanningRequest::ListActive { reply } => {
                        reply
                            .send(vec![active_record.clone()])
                            .expect("snapshot reply");
                        if let Some(sender) = synchronized_sender.take() {
                            sender.send(()).expect("synchronization observer");
                        }
                    }
                    _ => panic!("unexpected banning request"),
                }
            }
        });

        let local_view = LocalBanView::new::<()>(None, configured_policy);
        let service_available = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));
        let available_for_attempt = Arc::clone(&service_available);
        let attempts_for_attempt = Arc::clone(&attempts);
        let api_for_attempt = api.clone();
        local_view.start_synchronizer_with_acquisition::<(), _, _>(move || {
            attempts_for_attempt.fetch_add(1, Ordering::SeqCst);
            let available = available_for_attempt.load(Ordering::SeqCst);
            let api = api_for_attempt.clone();
            async move {
                if available {
                    Ok(api)
                } else {
                    Err("service not started".to_owned())
                }
            }
        });
        let view = ChainSyncBanView::new(local_view.clone());
        let candidates = HashSet::from([configured_peer, dynamic_peer, healthy_peer]);

        tokio::task::yield_now().await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert!(view.is_banned(configured_peer));
        assert_eq!(
            view.filter(&candidates),
            HashSet::from([dynamic_peer, healthy_peer])
        );

        service_available.store(true, Ordering::SeqCst);
        tokio::time::advance(Duration::from_secs(1)).await;
        synchronized_receiver
            .await
            .expect("view synchronizes after service becomes available");

        assert!(view.is_banned(dynamic_peer));
        assert_eq!(view.filter(&candidates), HashSet::from([healthy_peer]));
        handler.abort();
    }
}
