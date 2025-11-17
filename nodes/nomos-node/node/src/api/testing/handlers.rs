use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use axum::{Json, extract::State, response::Response};
use kzgrs_backend::common::share::DaShare;
use nomos_api::http::{da, mantle};
use nomos_core::{
    header::HeaderId,
    mantle::{SignedMantleTx, TxHash},
    sdp::SessionNumber,
};
use nomos_da_network_service::{
    NetworkService, api::ApiAdapter as ApiAdapterTrait, backends::NetworkBackend,
    sdp::SdpAdapter as SdpAdapterTrait,
};
use nomos_da_sampling::{
    DaSamplingService,
    backend::{DaSamplingServiceBackend, kzgrs::KzgrsSamplingBackend},
    mempool::DaMempoolAdapter,
    network::adapters::validator::Libp2pAdapter as SamplingLibp2pAdapter,
    storage::adapters::rocksdb::{
        RocksAdapter as SamplingStorageAdapter, converter::DaStorageConverter,
    },
};
use nomos_time::backends::NtpTimeBackend;
use overwatch::{overwatch::OverwatchHandle, services::AsServiceId};
use serde::{Deserialize, Serialize};
use subnetworks_assignations::MembershipHandler;
use tx_service::storage::adapters::rocksdb::RocksStorageAdapter;

use super::backend::TestHttpCryptarchiaService;
use crate::{
    DaMembershipStorage, DaNetworkApiAdapter, NomosDaMembership,
    generic_services::{
        DaMembershipAdapter, SdpService, SdpServiceAdapterGeneric, TxMempoolService,
    },
    make_request_and_return_response,
};

pub async fn da_get_membership<
    Backend,
    Membership,
    MembershipAdapter,
    MembershipStorage,
    ApiAdapter,
    SdpAdapter,
    RuntimeServiceId,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(session_id): Json<SessionNumber>,
) -> Response
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Membership: MembershipHandler + Clone + Send + Sync + 'static,
    Membership::Id: Send + Sync + 'static,
    Membership::NetworkId: Send + Sync + 'static,
    ApiAdapter: ApiAdapterTrait + Send + Sync + 'static,
    SdpAdapter: SdpAdapterTrait<RuntimeServiceId> + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + 'static
        + AsServiceId<
            NetworkService<
                Backend,
                Membership,
                MembershipAdapter,
                MembershipStorage,
                ApiAdapter,
                SdpAdapter,
                RuntimeServiceId,
            >,
        >,
{
    make_request_and_return_response!(da::da_get_membership::<
        Backend,
        Membership,
        MembershipAdapter,
        MembershipStorage,
        ApiAdapter,
        SdpAdapter,
        RuntimeServiceId,
    >(handle, session_id))
}

#[derive(Serialize, Deserialize)]
pub struct HistoricSamplingRequest<BlobId>
where
    BlobId: Eq + Hash,
{
    pub block_id: HeaderId,
    pub blob_ids: Vec<(BlobId, SessionNumber)>,
}

pub async fn da_historic_sampling<
    SamplingBackend,
    SamplingNetwork,
    SamplingStorage,
    SamplingMempoolAdapter,
    RuntimeServiceId,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(request): Json<HistoricSamplingRequest<SamplingBackend::BlobId>>,
) -> Response
where
    SamplingBackend: DaSamplingServiceBackend,
    <SamplingBackend as DaSamplingServiceBackend>::BlobId: Send + Eq + Hash + 'static,
    SamplingNetwork: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    SamplingMempoolAdapter: DaMempoolAdapter,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + AsServiceId<
            DaSamplingService<
                SamplingBackend,
                SamplingNetwork,
                SamplingStorage,
                SamplingMempoolAdapter,
                RuntimeServiceId,
            >,
        >,
{
    make_request_and_return_response!(da::da_historic_sampling::<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        SamplingMempoolAdapter,
        RuntimeServiceId,
    >(
        handle,
        request.block_id,
        request.blob_ids.into_iter().collect()
    ))
}

pub async fn get_sdp_declarations<RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
) -> Response
where
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<TestHttpCryptarchiaService<RuntimeServiceId>>
        + AsServiceId<SdpService<RuntimeServiceId>>
        + AsServiceId<TxMempoolService<RuntimeServiceId>>,
{
    make_request_and_return_response!(mantle::get_sdp_declarations::<
        KzgrsSamplingBackend,
        SamplingLibp2pAdapter<
            NomosDaMembership,
            DaMembershipAdapter<RuntimeServiceId>,
            DaMembershipStorage,
            DaNetworkApiAdapter,
            SdpServiceAdapterGeneric<RuntimeServiceId>,
            RuntimeServiceId,
        >,
        SamplingStorageAdapter<DaShare, DaStorageConverter>,
        RocksStorageAdapter<SignedMantleTx, TxHash>,
        NtpTimeBackend,
        RuntimeServiceId,
    >(&handle))
}
