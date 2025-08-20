use std::fmt::{Debug, Display};

use axum::{extract::State, response::Response, Json};
use kzgrs_backend::dispersal::Metadata;
use nomos_api::http::da::{self, DaDispersal};
use nomos_da_dispersal::{adapters::network::DispersalNetworkAdapter, backend::DispersalBackend};
use nomos_da_network_core::SubnetworkId;
use nomos_http_api_common::{paths, types::DispersalRequest};
use nomos_libp2p::PeerId;
use nomos_node::make_request_and_return_response;
use overwatch::{overwatch::handle::OverwatchHandle, services::AsServiceId};
use serde::Serialize;
use subnetworks_assignations::MembershipHandler;

#[utoipa::path(
    post,
    path = paths::DISPERSE_DATA,
    responses(
        (status = 200, description = "Disperse data in DA network"),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn disperse_data<Backend, NetworkAdapter, Membership, RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(dispersal_req): Json<DispersalRequest<Metadata>>,
) -> Response
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    Backend: DispersalBackend<NetworkAdapter = NetworkAdapter> + Send + Sync + 'static,
    Backend::Settings: Clone + Send + Sync,
    Backend::BlobId: Serialize,
    NetworkAdapter: DispersalNetworkAdapter<SubnetworkId = Membership::NetworkId> + Send,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + AsServiceId<DaDispersal<Backend, NetworkAdapter, Membership, RuntimeServiceId>>,
{
    make_request_and_return_response!(da::disperse_data::<
        Backend,
        NetworkAdapter,
        Membership,
        RuntimeServiceId,
    >(&handle, dispersal_req.data))
}
