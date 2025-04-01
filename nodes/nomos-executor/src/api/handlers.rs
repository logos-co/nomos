use std::fmt::{Debug, Display};

use axum::{extract::State, response::Response, Json};
use nomos_api::http::da::{self, DaDispersal};
use nomos_core::da::blob::metadata;
use nomos_da_dispersal::{
    adapters::{mempool::DaMempoolAdapter, network::DispersalNetworkAdapter},
    backend::DispersalBackend,
};
use nomos_da_network_core::SubnetworkId;
use nomos_libp2p::PeerId;
use nomos_node::make_request_and_return_response;
use overwatch::{overwatch::handle::OverwatchHandle, services::AsServiceId};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use subnetworks_assignations::MembershipHandler;

use super::paths;

#[derive(Serialize, Deserialize)]
pub struct DispersalRequest<Metadata> {
    pub data: Vec<u8>,
    pub metadata: Metadata,
}

#[utoipa::path(
    post,
    path = paths::DISPERSE_DATA,
    responses(
        (status = 200, description = "Disperse data in DA network"),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn disperse_data<
    Backend,
    NetworkAdapter,
    MempoolAdapter,
    Membership,
    Metadata,
    RuntimeServiceId,
>(
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
    Backend: DispersalBackend<
            NetworkAdapter = NetworkAdapter,
            MempoolAdapter = MempoolAdapter,
            Metadata = Metadata,
        > + Send
        + Sync,
    Backend::Settings: Clone + Send + Sync,
    NetworkAdapter: DispersalNetworkAdapter<SubnetworkId = Membership::NetworkId> + Send,
    MempoolAdapter: DaMempoolAdapter,
    Metadata: DeserializeOwned + metadata::Metadata + Debug + Send + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + AsServiceId<
            DaDispersal<
                Backend,
                NetworkAdapter,
                MempoolAdapter,
                Membership,
                Metadata,
                RuntimeServiceId,
            >,
        >,
{
    make_request_and_return_response!(da::disperse_data::<
        Backend,
        NetworkAdapter,
        MempoolAdapter,
        Membership,
        Metadata,
        RuntimeServiceId,
    >(&handle, dispersal_req.data, dispersal_req.metadata))
}
