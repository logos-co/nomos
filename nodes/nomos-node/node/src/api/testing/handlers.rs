use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use axum::{Json, extract::State, response::Response};
use nomos_api::http::{
    da::{self},
    membership::{self, MembershipUpdateRequest},
};
use nomos_core::{header::HeaderId, sdp::SessionNumber};
use nomos_da_network_service::{
    NetworkService, api::ApiAdapter as ApiAdapterTrait, backends::NetworkBackend,
    sdp::SdpAdapter as SdpAdapterTrait,
};
use nomos_da_sampling::{DaSamplingService, backend::DaSamplingServiceBackend};
use nomos_membership_service::{
    MembershipService, adapters::sdp::SdpAdapter, backends::MembershipBackend,
};
use overwatch::{overwatch::OverwatchHandle, services::AsServiceId};
use serde::{Deserialize, Serialize};
use subnetworks_assignations::MembershipHandler;

use crate::make_request_and_return_response;

pub async fn update_membership<Backend, Sdp, StorageAdapter, RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(payload): Json<MembershipUpdateRequest>,
) -> Response
where
    Backend: MembershipBackend + Send + Sync + 'static,
    Sdp: SdpAdapter + Send + Sync + 'static,
    Backend::Settings: Clone,
    RuntimeServiceId: Send
        + Sync
        + Debug
        + Display
        + 'static
        + AsServiceId<MembershipService<Backend, Sdp, StorageAdapter, RuntimeServiceId>>,
{
    make_request_and_return_response!(membership::update_membership_handler::<
        Backend,
        Sdp,
        StorageAdapter,
        RuntimeServiceId,
    >(handle, payload))
}

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
