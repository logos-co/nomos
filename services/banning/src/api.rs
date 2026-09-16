use std::time::Duration;

use lb_libp2p::PeerId;
use overwatch::services::relay::OutboundRelay;
use thiserror::Error;
use tokio::{sync::broadcast, time::timeout};

use crate::{
    BanEvent, BanRecord, BanScope, BanSource, BanningRequest, OffenseKind, Violation,
    service::BanningService,
};

const API_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Error)]
pub enum BanningApiError {
    #[error("banning service relay is unavailable: {0}")]
    Relay(String),
    #[error("banning service request timed out")]
    Timeout,
    #[error("banning service response channel was closed")]
    ResponseClosed,
    #[error("a global ban requires explicit provenance")]
    GlobalBanRequiresSource,
    #[error("configuration provenance is reserved for configured blacklist policy")]
    ConfigurationSourceReserved,
    #[error("no mutable ban exists for the requested peer and scope")]
    NoMutableBan,
}

/// Async client API for the central banning service.
#[derive(Clone)]
pub struct BanningServiceApi<RuntimeServiceId> {
    relay: OutboundRelay<
        <BanningService<RuntimeServiceId> as overwatch::services::ServiceData>::Message,
    >,
}

impl<RuntimeServiceId> std::fmt::Debug for BanningServiceApi<RuntimeServiceId> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BanningServiceApi")
            .finish_non_exhaustive()
    }
}

impl<RuntimeServiceId> BanningServiceApi<RuntimeServiceId> {
    #[must_use]
    pub const fn new(
        relay: OutboundRelay<
            <BanningService<RuntimeServiceId> as overwatch::services::ServiceData>::Message,
        >,
    ) -> Self {
        Self { relay }
    }

    /// Erases the runtime service-id marker. The relay message is independent
    /// of that marker, which lets a consumer pass the API through a generic
    /// network command without adding a banning-service service-id bound.
    #[must_use]
    pub fn into_untyped(self) -> BanningServiceApi<()> {
        BanningServiceApi { relay: self.relay }
    }

    async fn request<T>(
        &self,
        request: impl FnOnce(tokio::sync::oneshot::Sender<T>) -> BanningRequest,
    ) -> Result<T, BanningApiError> {
        timeout(API_TIMEOUT, async {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            self.relay
                .send(request(sender))
                .await
                .map_err(|error| BanningApiError::Relay(error.to_string()))?;
            receiver.await.map_err(|_| BanningApiError::ResponseClosed)
        })
        .await
        .map_err(|_| BanningApiError::Timeout)?
    }

    pub async fn report(&self, violation: Violation) -> Result<Option<BanRecord>, BanningApiError> {
        if violation.source == BanSource::Configuration {
            return Err(BanningApiError::ConfigurationSourceReserved);
        }
        self.request(|reply| BanningRequest::BanPeer { violation, reply })
            .await
    }

    pub async fn ban_peer(
        &self,
        peer_id: PeerId,
        scope: BanScope,
        offense: OffenseKind,
        duration: Duration,
        context: Option<String>,
    ) -> Result<Option<BanRecord>, BanningApiError> {
        let BanScope::Service(subsystem) = scope else {
            return Err(BanningApiError::GlobalBanRequiresSource);
        };
        self.report(Violation::new(
            peer_id,
            BanSource::Service(subsystem.clone()),
            BanScope::Service(subsystem),
            offense,
            duration,
            context,
        ))
        .await
    }

    pub async fn ban_peer_with_source(
        &self,
        peer_id: PeerId,
        source: BanSource,
        scope: BanScope,
        offense: OffenseKind,
        duration: Duration,
        context: Option<String>,
    ) -> Result<Option<BanRecord>, BanningApiError> {
        self.report(Violation::new(
            peer_id, source, scope, offense, duration, context,
        ))
        .await
    }

    /// Replaces an existing mutable record for the report's exact peer and
    /// scope. This operation never creates a record; absence and immutable
    /// configured policy are returned as [`BanningApiError::NoMutableBan`].
    pub async fn replace_ban(&self, violation: Violation) -> Result<BanRecord, BanningApiError> {
        if violation.source == BanSource::Configuration {
            return Err(BanningApiError::ConfigurationSourceReserved);
        }
        self.request(|reply| BanningRequest::ReplaceBan { violation, reply })
            .await?
            .ok_or(BanningApiError::NoMutableBan)
    }

    pub async fn list_active(&self) -> Result<Vec<BanRecord>, BanningApiError> {
        self.request(|reply| BanningRequest::ListActive { reply })
            .await
    }

    pub async fn unban(
        &self,
        peer_id: PeerId,
        scope: Option<BanScope>,
    ) -> Result<bool, BanningApiError> {
        self.request(|reply| BanningRequest::UnbanPeer {
            peer_id,
            scope,
            reply,
        })
        .await
    }

    pub async fn subscribe(&self) -> Result<broadcast::Receiver<BanEvent>, BanningApiError> {
        self.request(|reply| BanningRequest::Subscribe { reply })
            .await
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;

    #[tokio::test]
    async fn convenience_api_requires_explicit_source_for_global_bans() {
        let (sender, _receiver) = mpsc::channel(1);
        let api = BanningServiceApi::<()>::new(OutboundRelay::new(sender));

        assert!(matches!(
            api.ban_peer(
                PeerId::random(),
                BanScope::Global,
                OffenseKind::ProtocolViolation,
                Duration::from_secs(1),
                None,
            )
            .await,
            Err(BanningApiError::GlobalBanRequiresSource)
        ));
    }

    #[tokio::test]
    async fn configuration_source_is_rejected_for_dynamic_operations() {
        let (sender, _receiver) = mpsc::channel(1);
        let api = BanningServiceApi::<()>::new(OutboundRelay::new(sender));
        let violation = Violation::new(
            PeerId::random(),
            BanSource::Configuration,
            BanScope::Global,
            OffenseKind::ProtocolViolation,
            Duration::from_secs(1),
            None,
        );

        assert!(matches!(
            api.report(violation.clone()).await,
            Err(BanningApiError::ConfigurationSourceReserved)
        ));
        assert!(matches!(
            api.replace_ban(violation).await,
            Err(BanningApiError::ConfigurationSourceReserved)
        ));
    }

    #[tokio::test]
    async fn replace_absence_is_returned_as_an_explicit_error() {
        let (sender, mut receiver) = mpsc::channel(1);
        let api = BanningServiceApi::<()>::new(OutboundRelay::new(sender));
        let handler = tokio::spawn(async move {
            let Some(BanningRequest::ReplaceBan { reply, .. }) = receiver.recv().await else {
                panic!("expected replace request");
            };
            reply.send(None).expect("replace reply");
        });
        let violation = Violation::new(
            PeerId::random(),
            BanSource::Service(crate::Subsystem::ChainSync),
            BanScope::Service(crate::Subsystem::ChainSync),
            OffenseKind::ProtocolViolation,
            Duration::from_secs(1),
            None,
        );

        assert!(matches!(
            api.replace_ban(violation).await,
            Err(BanningApiError::NoMutableBan)
        ));
        handler.await.expect("replace handler");
    }
}
