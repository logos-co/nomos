use libp2p::PeerId;
use nomos_core::block::BlockNumber;
use nomos_da_network_core::SubnetworkId;
use nomos_storage::{backends::StorageBackend, StorageMsg, StorageService};
use overwatch::{services::relay::OutboundRelay, DynError};

use crate::{membership::Assignations, storage::MembershipStorageAdapter};

pub struct RocksAdapter<Backend, RuntimeServiceId>
where
    Backend: StorageBackend + Send + Sync + 'static,
{
    storage_relay: OutboundRelay<
        <StorageService<Backend, RuntimeServiceId> as overwatch::services::ServiceData>::Message,
    >,
}

#[async_trait::async_trait]
impl<Backend, RuntimeServiceId> MembershipStorageAdapter<PeerId, SubnetworkId>
    for RocksAdapter<Backend, RuntimeServiceId>
where
    Backend: StorageBackend<Id = PeerId, NetworkId = SubnetworkId> + Send + Sync + 'static,
{
    type StorageService = StorageService<Backend, RuntimeServiceId>;

    fn new(
        relay: OutboundRelay<<Self::StorageService as overwatch::services::ServiceData>::Message>,
    ) -> Self {
        Self {
            storage_relay: relay,
        }
    }

    async fn store(
        &self,
        block_number: BlockNumber,
        assignations: Assignations<PeerId, SubnetworkId>,
    ) -> Result<(), DynError> {
        let store_assignations_msg =
            StorageMsg::store_assignations_request(block_number, assignations);
        self.storage_relay
            .send(store_assignations_msg)
            .await
            .map_err(|(e, _)| DynError::from(e))?;

        Ok(())
    }

    async fn get(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Assignations<PeerId, SubnetworkId>>, DynError> {
        let (reply_channel, reply_rx) = tokio::sync::oneshot::channel();
        let get_assignations_msg =
            StorageMsg::get_assignations_request(block_number, reply_channel);

        self.storage_relay
            .send(get_assignations_msg)
            .await
            .map_err(|(e, _)| DynError::from(e))?;

        reply_rx.await.map_err(DynError::from)
    }

    async fn prune(&self) {
        todo!()
    }
}
