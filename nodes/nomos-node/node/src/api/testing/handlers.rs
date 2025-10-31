use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use axum::{Json, extract::State, response::Response};
use nomos_api::http::{da, mantle};
use nomos_core::{
    header::HeaderId,
    mantle::{SignedMantleTx, Transaction},
    sdp::SessionNumber,
};
use nomos_da_network_service::{
    NetworkService, api::ApiAdapter as ApiAdapterTrait, backends::NetworkBackend,
    sdp::SdpAdapter as SdpAdapterTrait,
};
use nomos_da_sampling::{DaSamplingService, backend::DaSamplingServiceBackend};
use nomos_time::backends::TimeBackend;
use overwatch::{overwatch::OverwatchHandle, services::AsServiceId};
use serde::{Deserialize, Serialize};
use subnetworks_assignations::MembershipHandler;

use crate::make_request_and_return_response;

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
pub struct HistoricSamplingRequest<BlobId> {
    pub session_id: SessionNumber,
    pub block_id: HeaderId,
    pub blob_ids: Vec<BlobId>,
}

pub async fn da_historic_sampling<
    SamplingBackend,
    SamplingNetwork,
    SamplingStorage,
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
    RuntimeServiceId: Debug
        + Sync
        + Display
        + AsServiceId<
            DaSamplingService<SamplingBackend, SamplingNetwork, SamplingStorage, RuntimeServiceId>,
        >,
{
    make_request_and_return_response!(da::da_historic_sampling::<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        RuntimeServiceId,
    >(
        handle,
        request.session_id,
        request.block_id,
        request.blob_ids
    ))
}

pub async fn get_sdp_declarations<
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    StorageAdapter,
    TimeBackendImpl,
    RuntimeServiceId,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
) -> Response
where
    SamplingBackend: DaSamplingServiceBackend<BlobId = [u8; 32]> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
    SamplingStorage:
        nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync + 'static,
    StorageAdapter: tx_service::storage::MempoolStorageAdapter<
            RuntimeServiceId,
            Item = SignedMantleTx,
            Key = <SignedMantleTx as Transaction>::Hash,
        > + Send
        + Sync
        + Clone
        + 'static,
    StorageAdapter::Error: Debug,
    TimeBackendImpl: TimeBackend,
    TimeBackendImpl::Settings: Clone + Send + Sync,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<
            nomos_api::http::consensus::Cryptarchia<
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                StorageAdapter,
                TimeBackendImpl,
                RuntimeServiceId,
            >,
        >,
{
    make_request_and_return_response!(mantle::get_sdp_declarations::<
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        StorageAdapter,
        TimeBackendImpl,
        RuntimeServiceId,
    >(&handle))
}
