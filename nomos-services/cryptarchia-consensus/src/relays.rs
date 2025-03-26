use std::{
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
};

use nomos_blend_service::{
    network::NetworkAdapter as BlendNetworkAdapter, BlendService, ServiceMessage,
};
use nomos_core::{
    da::blob::{info::DispersedBlobInfo, BlobSelect},
    header::HeaderId,
    tx::TxSelect,
};
use nomos_da_sampling::{backend::DaSamplingServiceBackend, DaSamplingService};
use nomos_mempool::{
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as MempoolAdapter,
    DaMempoolService, TxMempoolService,
};
use nomos_network::{NetworkMsg, NetworkService};
use nomos_storage::{backends::StorageBackend, StorageMsg, StorageService};
use nomos_time::backends::TimeBackend as TimeBackendTrait;
use overwatch::{
    services::{relay::OutboundRelay, AsServiceId},
    OpaqueServiceStateHandle,
};
use rand::{RngCore, SeedableRng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    blend, network,
    storage::{adapters::StorageAdapter, StorageAdapter as StorageAdapterTrait},
    CryptarchiaConsensus, MempoolRelay, SamplingRelay,
};

type NetworkRelay<NetworkBackend, RuntimeServiceId> =
    OutboundRelay<NetworkMsg<NetworkBackend, RuntimeServiceId>>;
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
type StorageRelay<Storage> = OutboundRelay<StorageMsg<Storage>>;

pub struct CryptarchiaConsensusRelays<
    BlendAdapter,
    BS,
    ClPool,
    ClPoolAdapter,
    DaPool,
    DaPoolAdapter,
    NetworkAdapter,
    SamplingBackend,
    SamplingRng,
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
    Storage: StorageBackend + Send + Sync,
    SamplingRng: SeedableRng + RngCore,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng>,
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
        SamplingRng,
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
        SamplingRng,
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
    ClPool::Item: Debug + DeserializeOwned + Eq + Hash + Clone + Send + Sync + 'static,
    ClPool::Key: Debug + 'static,
    ClPool::Settings: Clone,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    DaPool: RecoverableMempool<BlockId = HeaderId>,
    DaPool::BlockId: Debug,
    DaPool::Item: Debug + DeserializeOwned + Eq + Hash + Clone + Send + Sync + 'static,
    DaPool::Key: Debug + 'static,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Settings: Clone,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    NetworkAdapter::Settings: Send,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = DaPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingRng: SeedableRng + RngCore,
    Storage: StorageBackend + Send + Sync + 'static,
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
            _phantom_data: PhantomData,
        }
    }

    #[expect(clippy::allow_attributes_without_reason)]
    #[expect(clippy::type_complexity)]
    pub async fn from_service_state_handle<
        SamplingNetworkAdapter,
        SamplingStorage,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
    >(
        state_handle: &OpaqueServiceStateHandle<
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
                SamplingRng,
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
                TimeBackend,
                ApiAdapter,
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
        SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
        SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
        DaVerifierStorage:
            nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
        DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
        DaVerifierBackend::Settings: Clone,
        DaVerifierNetwork:
            nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
        DaVerifierNetwork::Settings: Clone,
        TimeBackend: TimeBackendTrait,
        TimeBackend::Settings: Clone + Send + Sync,
        ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
        RuntimeServiceId: Debug
            + Sync
            + Send
            + Display
            + 'static
            + AsServiceId<NetworkService<NetworkAdapter::Backend, RuntimeServiceId>>
            + AsServiceId<
                BlendService<BlendAdapter::Backend, BlendAdapter::Network, RuntimeServiceId>,
            >
            + AsServiceId<TxMempoolService<ClPoolAdapter, ClPool, RuntimeServiceId>>
            + AsServiceId<
                DaMempoolService<
                    DaPoolAdapter,
                    DaPool,
                    SamplingBackend,
                    SamplingNetworkAdapter,
                    SamplingRng,
                    SamplingStorage,
                    DaVerifierBackend,
                    DaVerifierNetwork,
                    DaVerifierStorage,
                    ApiAdapter,
                    RuntimeServiceId,
                >,
            >
            + AsServiceId<
                DaSamplingService<
                    SamplingBackend,
                    SamplingNetworkAdapter,
                    SamplingRng,
                    SamplingStorage,
                    DaVerifierBackend,
                    DaVerifierNetwork,
                    DaVerifierStorage,
                    ApiAdapter,
                    RuntimeServiceId,
                >,
            >
            + AsServiceId<StorageService<Storage, RuntimeServiceId>>,
    {
        let network_relay = state_handle
            .overwatch_handle
            .relay::<NetworkService<_, _>>()
            .await
            .expect("Relay connection with NetworkService should succeed");

        let blend_relay = state_handle
            .overwatch_handle
            .relay::<BlendService<_, _, _>>()
            .await
            .expect(
                "Relay connection with nomos_blend_service::BlendService should
        succeed",
            );

        let cl_mempool_relay = state_handle
            .overwatch_handle
            .relay::<TxMempoolService<_, _, _>>()
            .await
            .expect("Relay connection with CL MemPoolService should succeed");

        let da_mempool_relay = state_handle
            .overwatch_handle
            .relay::<DaMempoolService<_, _, _, _, _, _, _, _, _, _, _>>()
            .await
            .expect("Relay connection with DA MemPoolService should succeed");

        let sampling_relay = state_handle
            .overwatch_handle
            .relay::<DaSamplingService<_, _, _, _, _, _, _, _, _>>()
            .await
            .expect("Relay connection with SamplingService should succeed");

        let storage_relay = state_handle
            .overwatch_handle
            .relay::<StorageService<_, _>>()
            .await
            .expect("Relay connection with StorageService should succeed");

        Self::new(
            network_relay,
            blend_relay,
            cl_mempool_relay,
            da_mempool_relay,
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
}
