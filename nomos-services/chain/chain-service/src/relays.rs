use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use broadcast_service::{BlockBroadcastMsg, BlockBroadcastService};
use nomos_core::{
    block::Block,
    da,
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, TxHash},
};
use nomos_da_sampling::{DaSamplingService, backend::DaSamplingServiceBackend};
use nomos_mempool::{
    TxMempoolService,
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as MempoolNetworkAdapter,
    storage::MempoolStorageAdapter,
};
use nomos_network::{NetworkService, message::BackendNetworkMsg};
use nomos_storage::{
    StorageMsg, StorageService, api::chain::StorageChainApi, backends::StorageBackend,
};
use nomos_time::{TimeService, backends::TimeBackend as TimeBackendTrait};
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{AsServiceId, relay::OutboundRelay},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    CryptarchiaConsensus, MempoolRelay, SamplingRelay,
    mempool::adapter::MempoolAdapter,
    network,
    storage::{StorageAdapter as _, adapters::StorageAdapter},
};

type NetworkRelay<NetworkBackend, RuntimeServiceId> =
    OutboundRelay<BackendNetworkMsg<NetworkBackend, RuntimeServiceId>>;
pub type BroadcastRelay = OutboundRelay<BlockBroadcastMsg>;
type ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId> = MempoolRelay<
    <ClPoolAdapter as MempoolNetworkAdapter<RuntimeServiceId>>::Payload,
    <ClPool as MemPool>::Item,
    <ClPool as MemPool>::Key,
>;
pub type StorageRelay<Storage> = OutboundRelay<StorageMsg<Storage>>;

pub struct CryptarchiaConsensusRelays<
    ClPool,
    ClPoolAdapter,
    NetworkAdapter,
    SamplingBackend,
    Storage,
    RuntimeServiceId,
> where
    ClPool: MemPool,
    ClPoolAdapter: MempoolNetworkAdapter<RuntimeServiceId>,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend,
{
    network_relay: NetworkRelay<
        <NetworkAdapter as network::NetworkAdapter<RuntimeServiceId>>::Backend,
        RuntimeServiceId,
    >,
    broadcast_relay: BroadcastRelay,
    mempool_adapter: MempoolAdapter<ClPool::Item, ClPool::Item>,
    storage_adapter: StorageAdapter<Storage, ClPool::Item, RuntimeServiceId>,
    sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
    _clpool_adapter: std::marker::PhantomData<ClPoolAdapter>,
}

impl<ClPool, ClPoolAdapter, NetworkAdapter, SamplingBackend, Storage, RuntimeServiceId>
    CryptarchiaConsensusRelays<
        ClPool,
        ClPoolAdapter,
        NetworkAdapter,
        SamplingBackend,
        Storage,
        RuntimeServiceId,
    >
where
    ClPool: RecoverableMempool<BlockId = HeaderId, Key = TxHash> + Send + Sync,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Item: Debug + Serialize + DeserializeOwned + Eq + Clone + Send + Sync + 'static,
    ClPool::Item: AuthenticatedMantleTx,
    ClPool::Settings: Clone + Send + Sync,
    ClPoolAdapter: MempoolNetworkAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>
        + Send
        + Sync,
    ClPoolAdapter::Settings: Send + Sync,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    NetworkAdapter::Settings: Send,
    NetworkAdapter::PeerId: Clone + Eq + Hash + Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item>> + TryInto<Block<ClPool::Item>>,
    <Storage as StorageChainApi>::Tx: TryFrom<ClPool::Item> + TryInto<ClPool::Item>,
    ClPool::Storage: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
{
    pub async fn new(
        network_relay: NetworkRelay<
            <NetworkAdapter as network::NetworkAdapter<RuntimeServiceId>>::Backend,
            RuntimeServiceId,
        >,
        broadcast_relay: BroadcastRelay,
        cl_mempool_relay: ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId>,
        sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
        storage_relay: StorageRelay<Storage>,
    ) -> Self {
        let storage_adapter =
            StorageAdapter::<Storage, ClPool::Item, RuntimeServiceId>::new(storage_relay).await;
        let mempool_adapter = MempoolAdapter::new(cl_mempool_relay);
        Self {
            network_relay,
            broadcast_relay,
            mempool_adapter,
            storage_adapter,
            sampling_relay,
            _clpool_adapter: std::marker::PhantomData,
        }
    }

    #[expect(clippy::allow_attributes_without_reason)]
    #[expect(clippy::type_complexity)]
    pub async fn from_service_resources_handle<
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
    >(
        service_resources_handle: &OpaqueServiceResourcesHandle<
            CryptarchiaConsensus<
                NetworkAdapter,
                ClPool,
                ClPoolAdapter,
                Storage,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                TimeBackend,
                RuntimeServiceId,
            >,
            RuntimeServiceId,
        >,
    ) -> Self
    where
        ClPool::Key: Send,
        NetworkAdapter::Settings: Sync + Send,
        SamplingNetworkAdapter:
            nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
        SamplingStorage:
            nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
        TimeBackend: TimeBackendTrait,
        TimeBackend::Settings: Clone + Send + Sync,
        RuntimeServiceId: Debug
            + Sync
            + Send
            + Display
            + 'static
            + AsServiceId<NetworkService<NetworkAdapter::Backend, RuntimeServiceId>>
            + AsServiceId<BlockBroadcastService<RuntimeServiceId>>
            + AsServiceId<
                TxMempoolService<
                    ClPoolAdapter,
                    SamplingNetworkAdapter,
                    SamplingStorage,
                    ClPool,
                    ClPool::Storage,
                    RuntimeServiceId,
                >,
            >
            + AsServiceId<
                DaSamplingService<
                    SamplingBackend,
                    SamplingNetworkAdapter,
                    SamplingStorage,
                    RuntimeServiceId,
                >,
            >
            + AsServiceId<StorageService<Storage, RuntimeServiceId>>
            + AsServiceId<TimeService<TimeBackend, RuntimeServiceId>>,
    {
        let network_relay = service_resources_handle
            .overwatch_handle
            .relay::<NetworkService<_, _>>()
            .await
            .expect("Relay connection with NetworkService should succeed");

        let broadcast_relay = service_resources_handle
            .overwatch_handle
            .relay::<BlockBroadcastService<_>>()
            .await
            .expect(
                "Relay connection with broadcast_service::BlockBroadcastService should
        succeed",
            );

        let cl_mempool_relay = service_resources_handle
            .overwatch_handle
            .relay::<TxMempoolService<_, _, _, _, _, _>>()
            .await
            .expect("Relay connection with CL MemPoolService should succeed");

        let sampling_relay = service_resources_handle
            .overwatch_handle
            .relay::<DaSamplingService<_, _, _, _>>()
            .await
            .expect("Relay connection with SamplingService should succeed");

        let storage_relay = service_resources_handle
            .overwatch_handle
            .relay::<StorageService<_, _>>()
            .await
            .expect("Relay connection with StorageService should succeed");

        Self::new(
            network_relay,
            broadcast_relay,
            cl_mempool_relay,
            sampling_relay,
            storage_relay,
        )
        .await
    }

    pub const fn network_relay(
        &self,
    ) -> &NetworkRelay<
        <NetworkAdapter as network::NetworkAdapter<RuntimeServiceId>>::Backend,
        RuntimeServiceId,
    > {
        &self.network_relay
    }

    pub const fn broadcast_relay(&self) -> &BroadcastRelay {
        &self.broadcast_relay
    }

    pub const fn mempool_adapter(&self) -> &MempoolAdapter<ClPool::Item, ClPool::Item> {
        &self.mempool_adapter
    }

    pub const fn sampling_relay(&self) -> &SamplingRelay<SamplingBackend::BlobId> {
        &self.sampling_relay
    }

    pub const fn storage_adapter(
        &self,
    ) -> &StorageAdapter<Storage, ClPool::Item, RuntimeServiceId> {
        &self.storage_adapter
    }
}
