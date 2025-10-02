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
use nomos_network::{NetworkService, message::BackendNetworkMsg};
use nomos_storage::{
    StorageMsg, StorageService, api::chain::StorageChainApi, backends::StorageBackend,
};
use nomos_time::{TimeService, backends::TimeBackend as TimeBackendTrait};
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{AsServiceId, relay::OutboundRelay},
};
use serde::{Serialize, de::DeserializeOwned};
use tx_service::{MempoolMsg, TxMempoolService, backend::RecoverableMempool};

use crate::{
    CryptarchiaConsensus, SamplingRelay, network,
    storage::{StorageAdapter as _, adapters::StorageAdapter},
};

type NetworkRelay<NetworkBackend, RuntimeServiceId> =
    OutboundRelay<BackendNetworkMsg<NetworkBackend, RuntimeServiceId>>;
pub type BroadcastRelay = OutboundRelay<BlockBroadcastMsg>;

type MempoolRelay<Mempool, MempoolNetAdapter, RuntimeServiceId> = OutboundRelay<
    MempoolMsg<
        HeaderId,
        <MempoolNetAdapter as tx_service::network::NetworkAdapter<RuntimeServiceId>>::Payload,
        <Mempool as tx_service::backend::Mempool>::Item,
        <Mempool as tx_service::backend::Mempool>::Key,
    >,
>;

pub type StorageRelay<Storage> = OutboundRelay<StorageMsg<Storage>>;

pub struct CryptarchiaConsensusRelays<
    Mempool,
    MempoolNetAdapter,
    NetworkAdapter,
    SamplingBackend,
    Storage,
    RuntimeServiceId,
> where
    Mempool: tx_service::backend::Mempool,
    MempoolNetAdapter: tx_service::network::NetworkAdapter<RuntimeServiceId>,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend,
{
    network_relay: NetworkRelay<NetworkAdapter::Backend, RuntimeServiceId>,
    broadcast_relay: BroadcastRelay,
    mempool_relay: MempoolRelay<Mempool, MempoolNetAdapter, RuntimeServiceId>,
    storage_adapter: StorageAdapter<Storage, Mempool::Item, RuntimeServiceId>,
    sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
}

impl<Mempool, MempoolNetAdapter, NetworkAdapter, SamplingBackend, Storage, RuntimeServiceId>
    CryptarchiaConsensusRelays<
        Mempool,
        MempoolNetAdapter,
        NetworkAdapter,
        SamplingBackend,
        Storage,
        RuntimeServiceId,
    >
where
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash>,
    Mempool::RecoveryState: Serialize + DeserializeOwned,
    Mempool::Item: Debug + Serialize + DeserializeOwned + Eq + Clone + Send + Sync + 'static,
    Mempool::Item: AuthenticatedMantleTx,
    Mempool::Settings: Clone,
    MempoolNetAdapter: tx_service::network::NetworkAdapter<
            RuntimeServiceId,
            Payload = Mempool::Item,
            Key = Mempool::Key,
        >,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    NetworkAdapter::Settings: Send,
    NetworkAdapter::PeerId: Clone + Eq + Hash + Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    Storage: StorageChainApi + StorageBackend + Send + Sync + 'static,
    Storage::Block: TryFrom<Block<Mempool::Item>> + TryInto<Block<Mempool::Item>>,
{
    pub async fn new(
        network_relay: NetworkRelay<NetworkAdapter::Backend, RuntimeServiceId>,
        broadcast_relay: BroadcastRelay,
        mempool_relay: MempoolRelay<Mempool, MempoolNetAdapter, RuntimeServiceId>,
        sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
        storage_relay: StorageRelay<Storage>,
    ) -> Self {
        let storage_adapter =
            StorageAdapter::<Storage, Mempool::Item, RuntimeServiceId>::new(storage_relay).await;
        Self {
            network_relay,
            broadcast_relay,
            mempool_relay,
            storage_adapter,
            sampling_relay,
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
                Mempool,
                MempoolNetAdapter,
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
        Mempool::Key: Send,
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
                    MempoolNetAdapter,
                    SamplingNetworkAdapter,
                    SamplingStorage,
                    Mempool,
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

        let mempool_relay = service_resources_handle
            .overwatch_handle
            .relay::<TxMempoolService<_, _, _, _, _>>()
            .await
            .expect("Relay connection with MempoolService should succeed");

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
            mempool_relay,
            sampling_relay,
            storage_relay,
        )
        .await
    }

    pub const fn network_relay(&self) -> &NetworkRelay<NetworkAdapter::Backend, RuntimeServiceId> {
        &self.network_relay
    }

    pub const fn broadcast_relay(&self) -> &BroadcastRelay {
        &self.broadcast_relay
    }

    pub const fn mempool_relay(
        &self,
    ) -> &MempoolRelay<Mempool, MempoolNetAdapter, RuntimeServiceId> {
        &self.mempool_relay
    }

    pub const fn sampling_relay(&self) -> &SamplingRelay<SamplingBackend::BlobId> {
        &self.sampling_relay
    }

    pub const fn storage_adapter(
        &self,
    ) -> &StorageAdapter<Storage, Mempool::Item, RuntimeServiceId> {
        &self.storage_adapter
    }
}
