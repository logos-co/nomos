use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
};

use nomos_blend_service::{
    message::ServiceMessage, network::NetworkAdapter as BlendNetworkAdapter, BlendService,
};
use nomos_core::{
    block::Block,
    da::blob::{info::DispersedBlobInfo, BlobSelect},
    header::HeaderId,
    mantle::TxSelect,
};
use nomos_da_sampling::{backend::DaSamplingServiceBackend, DaSamplingService};
use nomos_mempool::{
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as MempoolAdapter,
    DaMempoolService, TxMempoolService,
};
use nomos_network::{message::BackendNetworkMsg, NetworkService};
use nomos_storage::{
    api::chain::StorageChainApi, backends::StorageBackend, StorageMsg, StorageService,
};
use nomos_time::{backends::TimeBackend as TimeBackendTrait, TimeService, TimeServiceMessage};
use overwatch::{
    services::{relay::OutboundRelay, AsServiceId},
    OpaqueServiceResourcesHandle,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    blend, network,
    storage::{adapters::StorageAdapter, StorageAdapter as _},
    CryptarchiaConsensus, MempoolRelay, SamplingRelay,
};

type NetworkRelay<NetworkBackend, RuntimeServiceId> =
    OutboundRelay<BackendNetworkMsg<NetworkBackend, RuntimeServiceId>>;
type BlendRelay<BlendAdapterNetworkBroadcastSettings> =
    OutboundRelay<ServiceMessage<BlendAdapterNetworkBroadcastSettings>>;
type ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId> = MempoolRelay<
    <ClPoolAdapter as MempoolAdapter<RuntimeServiceId>>::Payload,
    <ClPool as MemPool>::Item,
    <ClPool as MemPool>::Key,
>;
type DaMempoolRelay<DaPool, DaPoolAdapter, SamplingBackendBlobId, RuntimeServiceId> = MempoolRelay<
    <DaPoolAdapter as MempoolAdapter<RuntimeServiceId>>::Payload,
    <DaPool as MemPool>::Item,
    SamplingBackendBlobId,
>;
pub type StorageRelay<Storage> = OutboundRelay<StorageMsg<Storage>>;
type TimeRelay = OutboundRelay<TimeServiceMessage>;

pub struct CryptarchiaConsensusRelays<
    BlendAdapter,
    BS,
    ClPool,
    ClPoolAdapter,
    DaPool,
    DaPoolAdapter,
    NetworkAdapter,
    SamplingBackend,
    Storage,
    TxS,
    DaVerifierBackend,
    RuntimeServiceId,
> where
    BlendAdapter:
        blend::BlendAdapter<RuntimeServiceId, Network: BlendNetworkAdapter<RuntimeServiceId>>,
    BS: BlobSelect,
    ClPool: MemPool,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId>,
    DaPool: MemPool,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId>,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend,
    TxS: TxSelect,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend,
{
    network_relay: NetworkRelay<
        <NetworkAdapter as network::NetworkAdapter<RuntimeServiceId>>::Backend,
        RuntimeServiceId,
    >,
    blend_relay: BlendRelay<
        <<BlendAdapter as blend::BlendAdapter<RuntimeServiceId>>::Network as BlendNetworkAdapter<
            RuntimeServiceId,
        >>::BroadcastSettings,
    >,
    cl_mempool_relay: ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId>,
    da_mempool_relay:
        DaMempoolRelay<DaPool, DaPoolAdapter, SamplingBackend::BlobId, RuntimeServiceId>,
    storage_adapter: StorageAdapter<Storage, TxS::Tx, BS::BlobId, RuntimeServiceId>,
    sampling_relay: SamplingRelay<DaPool::Key>,
    time_relay: TimeRelay,
    _phantom_data: PhantomData<DaVerifierBackend>,
}

impl<
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetworkAdapter,
        SamplingBackend,
        Storage,
        TxS,
        DaVerifierBackend,
        RuntimeServiceId,
    >
    CryptarchiaConsensusRelays<
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetworkAdapter,
        SamplingBackend,
        Storage,
        TxS,
        DaVerifierBackend,
        RuntimeServiceId,
    >
where
    BlendAdapter:
        blend::BlendAdapter<RuntimeServiceId, Network: BlendNetworkAdapter<RuntimeServiceId>>,
    BlendAdapter::Settings: Send,
    BS: BlobSelect<BlobId = DaPool::Item>,
    BS::Settings: Send,
    ClPool: RecoverableMempool<BlockId = HeaderId>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Item: Debug + Serialize + DeserializeOwned + Eq + Clone + Send + Sync + 'static,
    ClPool::Key: Debug + 'static,
    ClPool::Settings: Clone,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    DaPool: RecoverableMempool<BlockId = HeaderId>,
    DaPool::BlockId: Debug,
    DaPool::Item: Debug + Serialize + DeserializeOwned + Eq + Clone + Send + Sync + 'static,
    DaPool::Key: Debug + 'static,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Settings: Clone,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    NetworkAdapter::Settings: Send,
    SamplingBackend: DaSamplingServiceBackend<BlobId = DaPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item, DaPool::Item>> + TryInto<Block<ClPool::Item, DaPool::Item>>,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
{
    pub async fn new(
        network_relay: NetworkRelay<
            <NetworkAdapter as network::NetworkAdapter<RuntimeServiceId>>::Backend,
            RuntimeServiceId,
        >,
        blend_relay: BlendRelay<
            <<BlendAdapter as blend::BlendAdapter<RuntimeServiceId>>::Network as BlendNetworkAdapter<RuntimeServiceId>>::BroadcastSettings,>,
        cl_mempool_relay: ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId>,
        da_mempool_relay: DaMempoolRelay<
            DaPool,
            DaPoolAdapter,
            SamplingBackend::BlobId,
            RuntimeServiceId,
        >,
        sampling_relay: SamplingRelay<DaPool::Key>,
        storage_relay: StorageRelay<Storage>,
        time_relay: TimeRelay,
    ) -> Self {
        let storage_adapter =
            StorageAdapter::<Storage, TxS::Tx, BS::BlobId, RuntimeServiceId>::new(storage_relay)
                .await;
        Self {
            network_relay,
            blend_relay,
            cl_mempool_relay,
            da_mempool_relay,
            sampling_relay,
            storage_adapter,
            time_relay,
            _phantom_data: PhantomData,
        }
    }

    #[expect(clippy::allow_attributes_without_reason)]
    #[expect(clippy::type_complexity)]
    pub async fn from_service_resources_handle<
        SamplingNetworkAdapter,
        SamplingStorage,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
    >(
        service_resources_handle: &OpaqueServiceResourcesHandle<
            CryptarchiaConsensus<
                NetworkAdapter,
                BlendAdapter,
                ClPool,
                ClPoolAdapter,
                DaPool,
                DaPoolAdapter,
                TxS,
                BS,
                Storage,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
                TimeBackend,
                RuntimeServiceId,
            >,
            RuntimeServiceId,
        >,
    ) -> Self
    where
        ClPool::Key: Send,
        DaPool::Key: Send,
        TxS::Settings: Sync,
        BS::Settings: Sync,
        NetworkAdapter::Settings: Sync + Send,
        BlendAdapter::Settings: Sync,
        SamplingNetworkAdapter:
            nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
        SamplingStorage:
            nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
        DaVerifierStorage:
            nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
        DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
        DaVerifierBackend::Settings: Clone,
        DaVerifierNetwork:
            nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
        DaVerifierNetwork::Settings: Clone,
        TimeBackend: TimeBackendTrait,
        TimeBackend::Settings: Clone + Send + Sync,
        RuntimeServiceId: Debug
            + Sync
            + Send
            + Display
            + 'static
            + AsServiceId<NetworkService<NetworkAdapter::Backend, RuntimeServiceId>>
            + AsServiceId<
                BlendService<
                    BlendAdapter::Backend,
                    BlendAdapter::NodeId,
                    BlendAdapter::Network,
                    RuntimeServiceId,
                >,
            >
            + AsServiceId<
                TxMempoolService<
                    ClPoolAdapter,
                    SamplingNetworkAdapter,
                    DaVerifierNetwork,
                    SamplingStorage,
                    DaVerifierStorage,
                    ClPool,
                    RuntimeServiceId,
                >,
            >
            + AsServiceId<
                DaMempoolService<
                    DaPoolAdapter,
                    DaPool,
                    SamplingBackend,
                    SamplingNetworkAdapter,
                    SamplingStorage,
                    DaVerifierBackend,
                    DaVerifierNetwork,
                    DaVerifierStorage,
                    RuntimeServiceId,
                >,
            >
            + AsServiceId<
                DaSamplingService<
                    SamplingBackend,
                    SamplingNetworkAdapter,
                    SamplingStorage,
                    DaVerifierBackend,
                    DaVerifierNetwork,
                    DaVerifierStorage,
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
            .relay::<BlendService<_, _, _, _>>()
            .await
            .expect(
                "Relay connection with nomos_blend_service::BlendService should
        succeed",
            );

        let cl_mempool_relay = service_resources_handle
            .overwatch_handle
            .relay::<TxMempoolService<_, _, _, _, _, _, _>>()
            .await
            .expect("Relay connection with CL MemPoolService should succeed");

        let da_mempool_relay = service_resources_handle
            .overwatch_handle
            .relay::<DaMempoolService<_, _, _, _, _, _, _, _, _>>()
            .await
            .expect("Relay connection with DA MemPoolService should succeed");

        let sampling_relay = service_resources_handle
            .overwatch_handle
            .relay::<DaSamplingService<_, _, _, _, _, _, _>>()
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
            cl_mempool_relay,
            da_mempool_relay,
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

    pub const fn blend_relay(
        &self,
    ) -> &BlendRelay<
        <BlendAdapter::Network as BlendNetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
    > {
        &self.blend_relay
    }

    pub const fn cl_mempool_relay(
        &self,
    ) -> &ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId> {
        &self.cl_mempool_relay
    }

    pub const fn da_mempool_relay(
        &self,
    ) -> &DaMempoolRelay<DaPool, DaPoolAdapter, SamplingBackend::BlobId, RuntimeServiceId> {
        &self.da_mempool_relay
    }

    pub const fn sampling_relay(&self) -> &SamplingRelay<DaPool::Key> {
        &self.sampling_relay
    }

    pub const fn storage_adapter(
        &self,
    ) -> &StorageAdapter<Storage, TxS::Tx, BS::BlobId, RuntimeServiceId> {
        &self.storage_adapter
    }

    pub const fn time_relay(&self) -> &TimeRelay {
        &self.time_relay
    }
}
