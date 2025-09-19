use std::collections::{BTreeSet, HashMap};

use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{Locator, ProviderId, ServiceType},
};
use nomos_storage::{StorageMsg, StorageService, backends::StorageBackend};
use overwatch::{DynError, services::relay::OutboundRelay};

pub struct MembershipRocksAdapter<Backend, RuntimeServiceId>
where
    Backend: StorageBackend + Send + Sync + 'static,
{
    storage_relay: OutboundRelay<
        <StorageService<Backend, RuntimeServiceId> as overwatch::services::ServiceData>::Message,
    >,
}

#[async_trait::async_trait]
impl<Backend, RuntimeServiceId> crate::adapters::storage::MembershipStorageAdapter
    for MembershipRocksAdapter<Backend, RuntimeServiceId>
where
    Backend: StorageBackend + Send + Sync + 'static,
{
    type StorageService = StorageService<Backend, RuntimeServiceId>;

    fn new(
        relay: OutboundRelay<<Self::StorageService as overwatch::services::ServiceData>::Message>,
    ) -> Self {
        Self {
            storage_relay: relay,
        }
    }

    async fn save_active_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), DynError> {
        let msg =
            StorageMsg::save_active_session_request(service_type, session_id, providers.clone());
        self.storage_relay
            .send(msg)
            .await
            .map_err(|(e, _)| DynError::from(e))?;
        Ok(())
    }

    async fn load_active_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, DynError> {
        let (reply_channel, reply_rx) = tokio::sync::oneshot::channel();
        let msg = StorageMsg::load_active_session_request(service_type, reply_channel);
        self.storage_relay
            .send(msg)
            .await
            .map_err(|(e, _)| DynError::from(e))?;
        reply_rx.await.map_err(DynError::from)
    }

    async fn save_latest_block(&mut self, block_number: BlockNumber) -> Result<(), DynError> {
        let msg = StorageMsg::save_latest_block_request(block_number);
        self.storage_relay
            .send(msg)
            .await
            .map_err(|(e, _)| DynError::from(e))?;
        Ok(())
    }

    async fn load_latest_block(&mut self) -> Result<Option<BlockNumber>, DynError> {
        let (reply_channel, reply_rx) = tokio::sync::oneshot::channel();
        let msg = StorageMsg::load_latest_block_request(reply_channel);
        self.storage_relay
            .send(msg)
            .await
            .map_err(|(e, _)| DynError::from(e))?;
        reply_rx.await.map_err(DynError::from)
    }

    async fn save_forming_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), DynError> {
        let msg =
            StorageMsg::save_forming_session_request(service_type, session_id, providers.clone());
        self.storage_relay
            .send(msg)
            .await
            .map_err(|(e, _)| DynError::from(e))?;
        Ok(())
    }

    async fn load_forming_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, DynError> {
        let (reply_channel, reply_rx) = tokio::sync::oneshot::channel();
        let msg = StorageMsg::load_forming_session_request(service_type, reply_channel);
        self.storage_relay
            .send(msg)
            .await
            .map_err(|(e, _)| DynError::from(e))?;
        reply_rx.await.map_err(DynError::from)
    }
}
