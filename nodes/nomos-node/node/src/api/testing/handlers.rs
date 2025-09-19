use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use axum::{extract::State, response::Response, Json};
use nomos_api::http::{
    da::{self},
    membership::{self, MembershipUpdateRequest},
};
use nomos_core::{block::SessionNumber, header::HeaderId};
use nomos_da_network_service::{
    api::ApiAdapter as ApiAdapterTrait, backends::NetworkBackend, NetworkService,
};
use nomos_da_sampling::{backend::DaSamplingServiceBackend, DaSamplingService};
use nomos_membership::{adapters::sdp::SdpAdapter, backends::MembershipBackend, MembershipService};
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
