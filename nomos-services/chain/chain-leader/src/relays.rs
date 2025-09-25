use std::fmt::{Debug, Display};

use chain_service::api::CryptarchiaServiceData;
use nomos_core::{
    da,
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, TxHash},
};
use nomos_da_sampling::{DaSamplingService, backend::DaSamplingServiceBackend};
use nomos_mempool::{
    TxMempoolService,
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as MempoolAdapter,
};
use nomos_time::{TimeService, TimeServiceMessage, backends::TimeBackend as TimeBackendTrait};
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceData, relay::OutboundRelay},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{MempoolRelay, SamplingRelay};

type BlendRelay<BlendService> = OutboundRelay<<BlendService as ServiceData>::Message>;
type ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId> = MempoolRelay<
    <ClPoolAdapter as MempoolAdapter<RuntimeServiceId>>::Payload,
    <ClPool as MemPool>::Item,
    <ClPool as MemPool>::Key,
>;
type TimeRelay = OutboundRelay<TimeServiceMessage>;

pub struct CryptarchiaConsensusRelays<
    BlendService,
    ClPool,
    ClPoolAdapter,
    SamplingBackend,
    RuntimeServiceId,
> where
    BlendService: ServiceData,
    ClPool: MemPool,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId>,
    SamplingBackend: DaSamplingServiceBackend,
{
    blend_relay: BlendRelay<BlendService>,
    cl_mempool_relay: ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId>,
    sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
    time_relay: TimeRelay,
}

impl<BlendService, ClPool, ClPoolAdapter, SamplingBackend, RuntimeServiceId>
    CryptarchiaConsensusRelays<
        BlendService,
        ClPool,
        ClPoolAdapter,
        SamplingBackend,
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
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
{
    pub const fn new(
        blend_relay: BlendRelay<BlendService>,
        cl_mempool_relay: ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId>,
        sampling_relay: SamplingRelay<SamplingBackend::BlobId>,
        time_relay: TimeRelay,
    ) -> Self {
        Self {
            blend_relay,
            cl_mempool_relay,
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
        ClPool::Key: Send,
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
            + AsServiceId<TimeService<TimeBackend, RuntimeServiceId>>
            + AsServiceId<CryptarchiaService>,
        CryptarchiaService: CryptarchiaServiceData<ClPool::Item>,
    {
        let blend_relay = service_resources_handle
            .overwatch_handle
            .relay::<BlendService>()
            .await
            .expect(
                "Relay connection with nomos_blend_service::BlendService should
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

        let time_relay = service_resources_handle
            .overwatch_handle
            .relay::<TimeService<_, _>>()
            .await
            .expect("Relay connection with TimeService should succeed");

        Self::new(blend_relay, cl_mempool_relay, sampling_relay, time_relay)
    }

    pub const fn blend_relay(&self) -> &BlendRelay<BlendService> {
        &self.blend_relay
    }

    pub const fn cl_mempool_relay(
        &self,
    ) -> &ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId> {
        &self.cl_mempool_relay
    }

    pub const fn sampling_relay(&self) -> &SamplingRelay<SamplingBackend::BlobId> {
        &self.sampling_relay
    }

    pub const fn time_relay(&self) -> &TimeRelay {
        &self.time_relay
    }
}
