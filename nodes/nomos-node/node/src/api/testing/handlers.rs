use std::fmt::{Debug, Display};

use axum::{extract::State, response::Response, Json};
use nomos_api::http::{
    da::{self},
    membership::{self, MembershipUpdateRequest},
};
use nomos_core::block::BlockNumber;
use nomos_da_network_service::{
    api::ApiAdapter as ApiAdapterTrait, backends::NetworkBackend, NetworkService,
};
use nomos_membership::{adapters::SdpAdapter, backends::MembershipBackend, MembershipService};
use overwatch::{overwatch::OverwatchHandle, services::AsServiceId};
use subnetworks_assignations::MembershipHandler;

use crate::make_request_and_return_response;

pub async fn update_membership<Backend, Sdp, RuntimeServiceId>(
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
        + AsServiceId<MembershipService<Backend, Sdp, RuntimeServiceId>>,
{
    make_request_and_return_response!(membership::update_membership_handler::<
        Backend,
        Sdp,
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
    Json(block_number): Json<BlockNumber>,
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
    >(handle, block_number))
}
