use std::fmt::{Debug, Display};

use chain_service::api::CryptarchiaServiceData;
use nomos_core::{
    da,
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, TxHash},
};
use nomos_da_sampling::{DaSamplingService, backend::DaSamplingServiceBackend};
use nomos_mempool::{
    MempoolMsg, TxMempoolService, backend::RecoverableMempool,
    network::NetworkAdapter as MempoolAdapter,
};
use nomos_time::{TimeService, TimeServiceMessage, backends::TimeBackend as TimeBackendTrait};
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceData, relay::OutboundRelay},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::SamplingRelay;

type BlendRelay<BlendService> = OutboundRelay<<BlendService as ServiceData>::Message>;

type MempoolRelay<Mempool, MempoolNetAdapter, RuntimeServiceId> = OutboundRelay<
    MempoolMsg<
        HeaderId,
        <MempoolNetAdapter as MempoolAdapter<RuntimeServiceId>>::Payload,
        <Mempool as nomos_mempool::backend::Mempool>::Item,
        <Mempool as nomos_mempool::backend::Mempool>::Key,
    >,
>;
type TimeRelay = OutboundRelay<TimeServiceMessage>;

pub struct CryptarchiaConsensusRelays<
    BlendService,
    Mempool,
    MempoolNetAdapter,
    SamplingBackend,
    RuntimeServiceId,
> where
    BlendService: ServiceData,
    Mempool: nomos_mempool::backend::Mempool,
    MempoolNetAdapter: MempoolAdapter<RuntimeServiceId>,
    SamplingBackend: DaSamplingServiceBackend,
{
    blend_relay: BlendRelay<BlendService>,
    mempool_relay: MempoolRelay<Mempool, MempoolNetAdapter, RuntimeServiceId>,
    sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
    time_relay: TimeRelay,
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
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash>,
    Mempool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    Mempool::Item: Debug + Serialize + DeserializeOwned + Eq + Clone + Send + Sync + 'static,
    Mempool::Item: AuthenticatedMantleTx,
    Mempool::Settings: Clone,
    MempoolNetAdapter:
        MempoolAdapter<RuntimeServiceId, Payload = Mempool::Item, Key = Mempool::Key>,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
{
    pub const fn new(
        blend_relay: BlendRelay<BlendService>,
        mempool_relay: MempoolRelay<Mempool, MempoolNetAdapter, RuntimeServiceId>,
        sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
        time_relay: TimeRelay,
    ) -> Self {
        Self {
            blend_relay,
            mempool_relay,
            sampling_relay,
            time_relay,
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
            + AsServiceId<TimeService<TimeBackend, RuntimeServiceId>>
            + AsServiceId<CryptarchiaService>,
        CryptarchiaService: CryptarchiaServiceData<Mempool::Item>,
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
            .relay::<TxMempoolService<_, _, _, _, _>>()
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

    pub const fn mempool_relay(
        &self,
    ) -> &MempoolRelay<Mempool, MempoolNetAdapter, RuntimeServiceId> {
        &self.mempool_relay
    }

    pub const fn sampling_relay(&self) -> &SamplingRelay<SamplingBackend::BlobId> {
        &self.sampling_relay
    }

    pub const fn time_relay(&self) -> &TimeRelay {
        &self.time_relay
    }
}
