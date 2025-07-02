use std::fmt::{Debug, Display};

use axum::{extract::State, response::Response, Json};
use nomos_api::http::membership::{self, MembershipUpdateRequest};
use nomos_membership::{adapters::SdpAdapter, backends::MembershipBackend, MembershipService};
use overwatch::overwatch::OverwatchHandle;

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
        + overwatch::services::AsServiceId<MembershipService<Backend, Sdp, RuntimeServiceId>>,
{
    make_request_and_return_response!(membership::update_membership_handler::<
        Backend,
        Sdp,
        RuntimeServiceId,
    >(handle, payload))
}
