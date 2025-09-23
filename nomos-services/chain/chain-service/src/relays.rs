use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use broadcast_service::{BlockBroadcastMsg, BlockBroadcastService};
use nomos_core::{
    block::Block,
    da,
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, TxHash, TxSelect},
};
use nomos_da_sampling::{DaSamplingService, backend::DaSamplingServiceBackend};
use nomos_mempool::{
    TxMempoolService,
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as MempoolAdapter,
};
use nomos_network::{NetworkService, message::BackendNetworkMsg};
use nomos_storage::{
    StorageMsg, StorageService, api::chain::StorageChainApi, backends::StorageBackend,
};
use nomos_time::{TimeService, TimeServiceMessage, backends::TimeBackend as TimeBackendTrait};
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceData, relay::OutboundRelay},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    CryptarchiaConsensus, MempoolRelay, SamplingRelay, network,
    storage::{StorageAdapter as _, adapters::StorageAdapter},
};

type NetworkRelay<NetworkBackend, RuntimeServiceId> =
    OutboundRelay<BackendNetworkMsg<NetworkBackend, RuntimeServiceId>>;
type BlendRelay<BlendService> = OutboundRelay<<BlendService as ServiceData>::Message>;
pub type BroadcastRelay = OutboundRelay<BlockBroadcastMsg>;
type ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId> = MempoolRelay<
    <ClPoolAdapter as MempoolAdapter<RuntimeServiceId>>::Payload,
    <ClPool as MemPool>::Item,
    <ClPool as MemPool>::Key,
>;
pub type StorageRelay<Storage> = OutboundRelay<StorageMsg<Storage>>;
type TimeRelay = OutboundRelay<TimeServiceMessage>;

pub struct CryptarchiaConsensusRelays<
    BlendService,
    ClPool,
    ClPoolAdapter,
    NetworkAdapter,
    SamplingBackend,
    Storage,
    TxS,
    RuntimeServiceId,
> where
    BlendService: ServiceData,
    ClPool: MemPool,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId>,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend,
    TxS: TxSelect,
{
    network_relay: NetworkRelay<
        <NetworkAdapter as network::NetworkAdapter<RuntimeServiceId>>::Backend,
        RuntimeServiceId,
    >,
    blend_relay: BlendRelay<BlendService>,
    broadcast_relay: BroadcastRelay,
    cl_mempool_relay: ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId>,
    storage_adapter: StorageAdapter<Storage, TxS::Tx, RuntimeServiceId>,
    sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
    time_relay: TimeRelay,
}

impl<
    BlendService,
    ClPool,
    ClPoolAdapter,
    NetworkAdapter,
    SamplingBackend,
    Storage,
    TxS,
    RuntimeServiceId,
>
    CryptarchiaConsensusRelays<
        BlendService,
        ClPool,
        ClPoolAdapter,
        NetworkAdapter,
        SamplingBackend,
        Storage,
        TxS,
        RuntimeServiceId,
    >
where
    BlendService: ServiceData,
    ClPool: RecoverableMempool<BlockId = HeaderId, Key = TxHash>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Item: Debug + Serialize + DeserializeOwned + Eq + Clone + Send + Sync + 'static,
    ClPool::Item: AuthenticatedMantleTx,
    ClPool::Settings: Clone,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    NetworkAdapter::Settings: Send,
    NetworkAdapter::PeerId: Clone + Eq + Hash + Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item>> + TryInto<Block<ClPool::Item>>,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send,
{
    pub async fn new(
        network_relay: NetworkRelay<
            <NetworkAdapter as network::NetworkAdapter<RuntimeServiceId>>::Backend,
            RuntimeServiceId,
        >,
        blend_relay: BlendRelay<BlendService>,
        broadcast_relay: BroadcastRelay,
        cl_mempool_relay: ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId>,
        sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
        storage_relay: StorageRelay<Storage>,
        time_relay: TimeRelay,
    ) -> Self {
        let storage_adapter =
            StorageAdapter::<Storage, TxS::Tx, RuntimeServiceId>::new(storage_relay).await;
        Self {
            network_relay,
            blend_relay,
            broadcast_relay,
            cl_mempool_relay,
            storage_adapter,
            sampling_relay,
            time_relay,
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
                BlendService,
                ClPool,
                ClPoolAdapter,
                TxS,
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
        TxS::Settings: Sync,
        NetworkAdapter::Settings: Sync + Send,
        BlendService: nomos_blend_service::ServiceComponents,
        BlendService::BroadcastSettings: Send + Sync,
        <BlendService as ServiceData>::Message: Send + 'static,
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
            + AsServiceId<BlendService>
            + AsServiceId<BlockBroadcastService<RuntimeServiceId>>
            + AsServiceId<
                TxMempoolService<
                    ClPoolAdapter,
                    SamplingNetworkAdapter,
                    SamplingStorage,
                    ClPool,
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

        let blend_relay = service_resources_handle
            .overwatch_handle
            .relay::<BlendService>()
            .await
            .expect(
                "Relay connection with nomos_blend_service::BlendService should
        succeed",
            );

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
            .relay::<TxMempoolService<_, _, _, _, _>>()
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

        let time_relay = service_resources_handle
            .overwatch_handle
            .relay::<TimeService<_, _>>()
            .await
            .expect("Relay connection with TimeService should succeed");

        Self::new(
            network_relay,
            blend_relay,
            broadcast_relay,
            cl_mempool_relay,
            sampling_relay,
            storage_relay,
            time_relay,
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

    pub const fn blend_relay(&self) -> &BlendRelay<BlendService> {
        &self.blend_relay
    }

    pub const fn broadcast_relay(&self) -> &BroadcastRelay {
        &self.broadcast_relay
    }

    pub const fn cl_mempool_relay(
        &self,
    ) -> &ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId> {
        &self.cl_mempool_relay
    }

    pub const fn sampling_relay(&self) -> &SamplingRelay<SamplingBackend::BlobId> {
        &self.sampling_relay
    }

    pub const fn storage_adapter(&self) -> &StorageAdapter<Storage, TxS::Tx, RuntimeServiceId> {
        &self.storage_adapter
    }

    pub const fn time_relay(&self) -> &TimeRelay {
        &self.time_relay
    }
}
