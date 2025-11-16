use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
};

use broadcast_service::{BlockBroadcastMsg, BlockBroadcastService};
use bytes::Bytes;
use nomos_core::{
    block::Block,
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, TxHash},
};
use nomos_storage::{
    StorageMsg, StorageService, api::chain::StorageChainApi, backends::StorageBackend,
};
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{AsServiceId, relay::OutboundRelay},
};
use serde::{Serialize, de::DeserializeOwned};
use tx_service::{
    MempoolMsg, TxMempoolService, backend::RecoverableMempool,
    network::NetworkAdapter as MempoolNetworkAdapter, storage::MempoolStorageAdapter,
};

use crate::{
    CryptarchiaConsensus,
    mempool::adapter::MempoolAdapter,
    storage::{StorageAdapter as _, adapters::StorageAdapter},
};

pub type BroadcastRelay = OutboundRelay<BlockBroadcastMsg>;

pub type StorageRelay<Storage> = OutboundRelay<StorageMsg<Storage>>;

pub struct CryptarchiaConsensusRelays<Mempool, MempoolNetAdapter, Storage, RuntimeServiceId>
where
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash> + Send + Sync,
    MempoolNetAdapter: tx_service::network::NetworkAdapter<RuntimeServiceId>,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Tx: From<Bytes> + AsRef<[u8]>,
{
    broadcast_relay: BroadcastRelay,
    mempool_adapter: MempoolAdapter<Mempool::Item, Mempool::Item>,
    storage_adapter: StorageAdapter<Storage, Mempool::Item, RuntimeServiceId>,
    _mempool_adapter: PhantomData<MempoolNetAdapter>,
}

impl<Mempool, MempoolNetAdapter, Storage, RuntimeServiceId>
    CryptarchiaConsensusRelays<Mempool, MempoolNetAdapter, Storage, RuntimeServiceId>
where
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash> + Send + Sync,
    Mempool::RecoveryState: Serialize + DeserializeOwned,
    Mempool::Item: Debug
        + Serialize
        + DeserializeOwned
        + Eq
        + Clone
        + Send
        + Sync
        + 'static
        + AuthenticatedMantleTx,
    Mempool::Settings: Clone + Send + Sync,
    Mempool::Storage: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
    MempoolNetAdapter: MempoolNetworkAdapter<RuntimeServiceId, Payload = Mempool::Item, Key = Mempool::Key>
        + Send
        + Sync,
    MempoolNetAdapter::Settings: Send + Sync,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Tx: From<Bytes> + AsRef<[u8]>,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<Mempool::Item>> + TryInto<Block<Mempool::Item>>,
{
    pub async fn new(
        broadcast_relay: BroadcastRelay,
        mempool_relay: OutboundRelay<MempoolMsg<HeaderId, Mempool::Item, Mempool::Item, TxHash>>,
        storage_relay: StorageRelay<Storage>,
    ) -> Self {
        let storage_adapter =
            StorageAdapter::<Storage, Mempool::Item, RuntimeServiceId>::new(storage_relay).await;
        let mempool_adapter = MempoolAdapter::new(mempool_relay);
        Self {
            broadcast_relay,
            mempool_adapter,
            storage_adapter,
            _mempool_adapter: PhantomData,
        }
    }

    #[expect(clippy::allow_attributes_without_reason)]
    pub async fn from_service_resources_handle(
        service_resources_handle: &OpaqueServiceResourcesHandle<
            CryptarchiaConsensus<Mempool, MempoolNetAdapter, Storage, RuntimeServiceId>,
            RuntimeServiceId,
        >,
    ) -> Self
    where
        Mempool::Key: Send,
        RuntimeServiceId: Debug
            + Sync
            + Send
            + Display
            + 'static
            + AsServiceId<BlockBroadcastService<RuntimeServiceId>>
            + AsServiceId<
                TxMempoolService<MempoolNetAdapter, Mempool, Mempool::Storage, RuntimeServiceId>,
            >
            + AsServiceId<StorageService<Storage, RuntimeServiceId>>,
    {
        let broadcast_relay = service_resources_handle
            .overwatch_handle
            .relay::<BlockBroadcastService<_>>()
            .await
            .expect(
                "Relay connection with broadcast_service::BlockBroadcastService should
        succeed",
            );

        let mempool_relay = service_resources_handle
            .overwatch_handle
            .relay::<TxMempoolService<_, _, _, _>>()
            .await
            .expect("Relay connection with MempoolService should succeed");

        let storage_relay = service_resources_handle
            .overwatch_handle
            .relay::<StorageService<_, _>>()
            .await
            .expect("Relay connection with StorageService should succeed");

        Self::new(broadcast_relay, mempool_relay, storage_relay).await
    }

    pub const fn broadcast_relay(&self) -> &BroadcastRelay {
        &self.broadcast_relay
    }

    pub const fn mempool_adapter(&self) -> &MempoolAdapter<Mempool::Item, Mempool::Item> {
        &self.mempool_adapter
    }

    pub const fn storage_adapter(
        &self,
    ) -> &StorageAdapter<Storage, Mempool::Item, RuntimeServiceId> {
        &self.storage_adapter
    }
}
