use std::{
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
};

use nomos_blend_service::{
    network::NetworkAdapter as BlendNetworkAdapter, BlendService, ServiceMessage,
};
use nomos_core::{
    block::Block,
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
use nomos_network::{message::BackendNetworkMsg, NetworkService};
use nomos_storage::{
    api::chain::StorageChainApi, backends::StorageBackend, StorageMsg, StorageService,
};
use nomos_time::{backends::TimeBackend as TimeBackendTrait, TimeService, TimeServiceMessage};
use overwatch::{
    services::{relay::OutboundRelay, AsServiceId},
    OpaqueServiceStateHandle,
};
use rand::{RngCore, SeedableRng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use nomos_core::da::BlobId;
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
type StorageRelay<Storage> = OutboundRelay<StorageMsg<Storage>>;
type TimeRelay = OutboundRelay<TimeServiceMessage>;

pub struct CryptarchiaConsensusRelays<
    BlendAdapter,
    ClPool,
    ClPoolAdapter,
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
    ClPool: MemPool,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId>,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingRng: SeedableRng + RngCore,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = BlobId>,
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
    storage_adapter: StorageAdapter<Storage, TxS::Tx, BlobId, RuntimeServiceId>,
    sampling_relay: SamplingRelay<BlobId>,
    time_relay: TimeRelay,
    _phantom_data: PhantomData<(DaVerifierBackend, SamplingBackend, SamplingRng)>,
}

impl<
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
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
        ClPool,
        ClPoolAdapter,
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
    ClPool: RecoverableMempool<BlockId = HeaderId>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Item: Debug + Serialize + DeserializeOwned + Eq + Hash + Clone + Send + Sync + 'static,
    ClPool::Key: Debug + 'static,
    ClPool::Settings: Clone,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    NetworkAdapter::Settings: Send,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingRng: SeedableRng + RngCore,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item>> + TryInto<Block<ClPool::Item>>,
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
        sampling_relay: SamplingRelay<BlobId>,
        storage_relay: StorageRelay<Storage>,
        time_relay: TimeRelay,
    ) -> Self {
        let storage_adapter =
            StorageAdapter::<Storage, TxS::Tx, BlobId, RuntimeServiceId>::new(storage_relay).await;
        Self {
            network_relay,
            blend_relay,
            cl_mempool_relay,
            sampling_relay,
            storage_adapter,
            time_relay,
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
                TxS,
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
        TxS::Settings: Sync,
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
            + AsServiceId<StorageService<Storage, RuntimeServiceId>>
            + AsServiceId<TimeService<TimeBackend, RuntimeServiceId>>,
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

        let time_relay = state_handle
            .overwatch_handle
            .relay::<TimeService<_, _>>()
            .await
            .expect("Relay connection with TimeService should succeed");

        Self::new(
            network_relay,
            blend_relay,
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

    pub const fn sampling_relay(&self) -> &SamplingRelay<BlobId> {
        &self.sampling_relay
    }

    pub const fn storage_adapter(
        &self,
    ) -> &StorageAdapter<Storage, TxS::Tx, BlobId, RuntimeServiceId> {
        &self.storage_adapter
    }

    pub const fn time_relay(&self) -> &TimeRelay {
        &self.time_relay
    }
}
