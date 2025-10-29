use std::fmt::{Debug, Display};

use chain_service::api::CryptarchiaServiceData;
use nomos_core::{
    da,
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, TxHash},
};
use nomos_da_sampling::{DaSamplingService, backend::DaSamplingServiceBackend};
use nomos_time::{TimeService, TimeServiceMessage, backends::TimeBackend as TimeBackendTrait};
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceData, relay::OutboundRelay},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tx_service::{
    MempoolMsg, TxMempoolService, backend::RecoverableMempool,
    network::NetworkAdapter as MempoolNetworkAdapter, storage::MempoolStorageAdapter,
};

use crate::{SamplingRelay, mempool::adapter};

type BlendRelay<BlendService> = OutboundRelay<<BlendService as ServiceData>::Message>;
type TimeRelay = OutboundRelay<TimeServiceMessage>;

pub struct CryptarchiaConsensusRelays<
    BlendService,
    Mempool,
    MempoolNetAdapter,
    SamplingBackend,
    RuntimeServiceId,
> where
    BlendService: ServiceData,
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash>,
    MempoolNetAdapter: MempoolNetworkAdapter<RuntimeServiceId>,
    SamplingBackend: DaSamplingServiceBackend,
{
    blend_relay: BlendRelay<BlendService>,
    mempool_adapter: adapter::MempoolAdapter<Mempool::Item, Mempool::Item>,
    sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
    time_relay: TimeRelay,
    _mempool_adapter: std::marker::PhantomData<(MempoolNetAdapter, RuntimeServiceId)>,
}

impl<BlendService, Mempool, MempoolNetAdapter, SamplingBackend, RuntimeServiceId>
    CryptarchiaConsensusRelays<
        BlendService,
        Mempool,
        MempoolNetAdapter,
        SamplingBackend,
        RuntimeServiceId,
    >
where
    BlendService: ServiceData,
    Mempool: Send + Sync + RecoverableMempool<BlockId = HeaderId, Key = TxHash>,
    Mempool::Storage: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
    Mempool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    Mempool::Item: Debug
        + Serialize
        + DeserializeOwned
        + Eq
        + Clone
        + Send
        + Sync
        + 'static
        + AuthenticatedMantleTx,
    Mempool::Settings: Clone,
    MempoolNetAdapter: MempoolNetworkAdapter<RuntimeServiceId, Payload = Mempool::Item, Key = Mempool::Key>
        + Send
        + Sync,
    MempoolNetAdapter::Settings: Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
{
    pub const fn new(
        blend_relay: BlendRelay<BlendService>,
        mempool_relay: OutboundRelay<MempoolMsg<HeaderId, Mempool::Item, Mempool::Item, TxHash>>,
        sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
        time_relay: TimeRelay,
    ) -> Self {
        let mempool_adapter = adapter::MempoolAdapter::new(mempool_relay);
        Self {
            blend_relay,
            mempool_adapter,
            sampling_relay,
            time_relay,
            _mempool_adapter: std::marker::PhantomData,
        }
    }

    #[expect(clippy::allow_attributes_without_reason)]
    pub async fn from_service_resources_handle<
        S,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        CryptarchiaService,
    >(
        service_resources_handle: &OpaqueServiceResourcesHandle<S, RuntimeServiceId>,
    ) -> Self
    where
        S: ServiceData,
        <S as ServiceData>::Message: Send + Sync + 'static,
        <S as ServiceData>::Settings: Send + Sync + 'static,
        <S as ServiceData>::State: Send + Sync + 'static,
        Mempool::Key: Send,
        Mempool::Storage: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
        Mempool::Settings: Sync,
        BlendService: nomos_blend_service::ServiceComponents,
        BlendService::BroadcastSettings: Send + Sync,
        <BlendService as ServiceData>::Message: Send + 'static,
        SamplingNetworkAdapter:
            nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
        SamplingStorage:
            nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
        TimeBackend: TimeBackendTrait,
        TimeBackend::Settings: Clone + Send + Sync + 'static,
        RuntimeServiceId: Debug
            + Sync
            + Send
            + Display
            + 'static
            + AsServiceId<BlendService>
            + AsServiceId<
                TxMempoolService<MempoolNetAdapter, Mempool, Mempool::Storage, RuntimeServiceId>,
            >
            + AsServiceId<
                DaSamplingService<
                    SamplingBackend,
                    SamplingNetworkAdapter,
                    SamplingStorage,
                    RuntimeServiceId,
                >,
            >
            + AsServiceId<TimeService<TimeBackend, RuntimeServiceId>>
            + AsServiceId<CryptarchiaService>,
        CryptarchiaService: CryptarchiaServiceData<Tx = Mempool::Item>,
    {
        let blend_relay = service_resources_handle
            .overwatch_handle
            .relay::<BlendService>()
            .await
            .expect(
                "Relay connection with nomos_blend_service::BlendService should
        succeed",
            );

        let mempool_relay = service_resources_handle
            .overwatch_handle
            .relay::<TxMempoolService<_, _, _, _>>()
            .await
            .expect("Relay connection with MempoolService should succeed");

        let sampling_relay = service_resources_handle
            .overwatch_handle
            .relay::<DaSamplingService<_, _, _, _>>()
            .await
            .expect("Relay connection with SamplingService should succeed");

        let time_relay = service_resources_handle
            .overwatch_handle
            .relay::<TimeService<_, _>>()
            .await
            .expect("Relay connection with TimeService should succeed");

        Self::new(blend_relay, mempool_relay, sampling_relay, time_relay)
    }

    pub const fn blend_relay(&self) -> &BlendRelay<BlendService> {
        &self.blend_relay
    }

    pub const fn mempool_adapter(&self) -> &adapter::MempoolAdapter<Mempool::Item, Mempool::Item> {
        &self.mempool_adapter
    }

    pub const fn sampling_relay(&self) -> &SamplingRelay<SamplingBackend::BlobId> {
        &self.sampling_relay
    }

    pub const fn time_relay(&self) -> &TimeRelay {
        &self.time_relay
    }
}
