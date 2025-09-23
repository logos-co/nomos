use std::{
    error::Error,
    fmt::{Debug, Display},
    hash::Hash,
};

use axum::{
    Json,
    body::Body,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse as _, Response},
};
use broadcast_service::BlockBroadcastService;
use futures::StreamExt as _;
use nomos_api::http::{
    DynError,
    cl::{self, ClMempoolService},
    consensus::{self, Cryptarchia},
    da::{self, BalancerMessageFactory, DaVerifier, MonitorMessageFactory},
    libp2p, mempool,
    storage::StorageAdapter,
};
use nomos_blend_service::ProofsVerifier;
use nomos_core::{
    da::{BlobId, DaVerifier as CoreDaVerifier, blob::Share},
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, SignedMantleTx, Transaction},
};
use nomos_da_messages::http::da::{
    DASharesCommitmentsRequest, DaSamplingRequest, GetSharesRequest,
};
use nomos_da_network_service::{
    NetworkService, api::ApiAdapter as ApiAdapterTrait, backends::NetworkBackend,
};
use nomos_da_sampling::{DaSamplingService, backend::DaSamplingServiceBackend};
use nomos_da_verifier::{backend::VerifierBackend, mempool::DaMempoolAdapter};
use nomos_http_api_common::paths;
use nomos_libp2p::PeerId;
use nomos_mempool::{
    TxMempoolService, backend::mockpool::MockPool,
    network::adapters::libp2p::Libp2pAdapter as MempoolNetworkAdapter,
};
use nomos_network::backends::libp2p::Libp2p as Libp2pNetworkBackend;
use nomos_storage::{StorageService, api::da::DaConverter, backends::rocksdb::RocksBackend};
use overwatch::{overwatch::handle::OverwatchHandle, services::AsServiceId};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use subnetworks_assignations::MembershipHandler;

use crate::api::backend::DaStorageBackend;

#[macro_export]
macro_rules! make_request_and_return_response {
    ($cond:expr) => {{
        match $cond.await {
            ::std::result::Result::Ok(val) => ::axum::response::IntoResponse::into_response((
                ::axum::http::StatusCode::OK,
                ::axum::Json(val),
            )),
            ::std::result::Result::Err(e) => ::axum::response::IntoResponse::into_response((
                ::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
            )),
        }
    }};
}

#[utoipa::path(
    get,
    path = paths::CL_METRICS,
    responses(
        (status = 200, description = "Get the mempool metrics of the cl service", body = MempoolMetrics),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn cl_metrics<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
) -> Response
where
    Tx: AuthenticatedMantleTx
        + Clone
        + Debug
        + Serialize
        + for<'de> Deserialize<'de>
        + Send
        + Sync
        + 'static,
    <Tx as Transaction>::Hash:
        Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<ClMempoolService<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>>,
{
    make_request_and_return_response!(cl::cl_mempool_metrics::<
        Tx,
        SamplingNetworkAdapter,
        SamplingStorage,
        RuntimeServiceId,
    >(&handle))
}

#[utoipa::path(
    post,
    path = paths::CL_STATUS,
    responses(
        (status = 200, description = "Query the mempool status of the cl service", body = Vec<<T as Transaction>::Hash>),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn cl_status<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(items): Json<Vec<<Tx as Transaction>::Hash>>,
) -> Response
where
    Tx: AuthenticatedMantleTx
        + Clone
        + Debug
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <Tx as Transaction>::Hash: Serialize + DeserializeOwned + Ord + Debug + Send + Sync + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<ClMempoolService<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>>,
{
    make_request_and_return_response!(cl::cl_mempool_status::<
        Tx,
        SamplingNetworkAdapter,
        SamplingStorage,
        RuntimeServiceId,
    >(&handle, items))
}
#[derive(Deserialize)]
pub struct CryptarchiaInfoQuery {
    from: Option<HeaderId>,
    to: Option<HeaderId>,
}

#[utoipa::path(
    get,
    path = paths::CRYPTARCHIA_INFO,
    responses(
        (status = 200, description = "Query consensus information", body = nomos_consensus::CryptarchiaInfo),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn cryptarchia_info<
    Tx,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    BlendProofsGenerator,
    BlendProofsVerifier,
    RuntimeServiceId,
    const SIZE: usize,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
) -> Response
where
    Tx: AuthenticatedMantleTx
        + Clone
        + Eq
        + Debug
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <Tx as Transaction>::Hash:
        Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    BlendProofsVerifier: ProofsVerifier + Clone + Send + 'static,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<
            Cryptarchia<
                Tx,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                TimeBackend,
                BlendProofsGenerator,
                BlendProofsVerifier,
                RuntimeServiceId,
                SIZE,
            >,
        >,
{
    make_request_and_return_response!(consensus::cryptarchia_info::<
        Tx,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        BlendProofsGenerator,
        BlendProofsVerifier,
        RuntimeServiceId,
        SIZE,
    >(&handle))
}

#[utoipa::path(
    get,
    path = paths::CRYPTARCHIA_HEADERS,
    responses(
        (status = 200, description = "Query header ids", body = Vec<HeaderId>),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn cryptarchia_headers<
    Tx,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    BlendProofsGenerator,
    BlendProofsVerifier,
    RuntimeServiceId,
    const SIZE: usize,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Query(query): Query<CryptarchiaInfoQuery>,
) -> Response
where
    Tx: AuthenticatedMantleTx
        + Eq
        + Clone
        + Debug
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <Tx as Transaction>::Hash:
        Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    BlendProofsVerifier: ProofsVerifier + Clone + Send + 'static,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<
            Cryptarchia<
                Tx,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                TimeBackend,
                BlendProofsGenerator,
                BlendProofsVerifier,
                RuntimeServiceId,
                SIZE,
            >,
        >,
{
    let CryptarchiaInfoQuery { from, to } = query;
    make_request_and_return_response!(consensus::cryptarchia_headers::<
        Tx,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        BlendProofsGenerator,
        BlendProofsVerifier,
        RuntimeServiceId,
        SIZE,
    >(&handle, from, to))
}

#[utoipa::path(
    get,
    path = paths::CRYPTARCHIA_LIB_STREAM,
    responses(
        (status = 200, description = "Request a stream for lib blocks"),
        (status = 500, description = "Internal server error", body = StreamBody),
    )
)]
pub async fn cryptarchia_lib_stream<RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
) -> Response
where
    RuntimeServiceId:
        Debug + Sync + Display + AsServiceId<BlockBroadcastService<RuntimeServiceId>> + 'static,
{
    match cl::lib_block_stream(&handle).await {
        Ok(shares) => {
            let stream = shares.map(|res| {
                let info = res?;
                let mut bytes = serde_json::to_vec(&info).map_err(|e| Box::new(e) as DynError)?;
                bytes.push(b'\n');
                Ok::<_, DynError>(bytes)
            });

            let body = Body::from_stream(stream);

            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/x-ndjson")
                .body(body)
                .unwrap()
                .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[utoipa::path(
    post,
    path = paths::DA_ADD_SHARE,
    responses(
        (status = 200, description = "Share to be published received"),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn add_share<S, N, VB, StorageConverter, VerifierMempoolAdapter, RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(share): Json<S>,
) -> Response
where
    S: Share + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    <S as Share>::BlobId: Clone + Send + Sync + 'static,
    <S as Share>::ShareIndex: Clone + Hash + Eq + Send + Sync + 'static,
    <S as Share>::LightShare: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    <S as Share>::SharesCommitments: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    N: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId>,
    N::Settings: Clone,
    VB: VerifierBackend + CoreDaVerifier<DaShare = S>,
    <VB as VerifierBackend>::Settings: Clone,
    <VB as CoreDaVerifier>::Error: Error,
    StorageConverter:
        DaConverter<DaStorageBackend, Share = S, Tx = SignedMantleTx> + Send + Sync + 'static,
    VerifierMempoolAdapter: DaMempoolAdapter + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + 'static
        + AsServiceId<
            DaVerifier<S, N, VB, StorageConverter, VerifierMempoolAdapter, RuntimeServiceId>,
        >,
{
    make_request_and_return_response!(da::add_share::<
        S,
        N,
        VB,
        StorageConverter,
        VerifierMempoolAdapter,
        RuntimeServiceId,
    >(&handle, share))
}

#[utoipa::path(
    post,
    path = paths::DA_BLOCK_PEER,
    responses(
        (status = 200, description = "Block a peer", body = bool),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn block_peer<
    Backend,
    Membership,
    MembershipAdapter,
    MembershipStorage,
    ApiAdapter,
    RuntimeServiceId,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(peer_id): Json<PeerId>,
) -> Response
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Backend::Message: MonitorMessageFactory,
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
    make_request_and_return_response!(da::block_peer::<
        Backend,
        Membership,
        MembershipAdapter,
        MembershipStorage,
        ApiAdapter,
        RuntimeServiceId,
    >(&handle, peer_id))
}

#[utoipa::path(
    post,
    path = paths::DA_UNBLOCK_PEER,
    responses(
        (status = 200, description = "Unblock a peer", body = bool),
        (status = 500, description = "Internal server error", body = String),
    )
)]

pub async fn unblock_peer<
    Backend,
    Membership,
    MembershipAdapter,
    MembershipStorage,
    ApiAdapter,
    RuntimeServiceId,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(peer_id): Json<PeerId>,
) -> Response
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Backend::Message: MonitorMessageFactory,
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
    make_request_and_return_response!(da::unblock_peer::<
        Backend,
        Membership,
        MembershipAdapter,
        MembershipStorage,
        ApiAdapter,
        RuntimeServiceId,
    >(&handle, peer_id))
}

#[utoipa::path(
    get,
    path = paths::DA_BLACKLISTED_PEERS,
    responses(
        (status = 200, description = "Get the blacklisted peers", body = Vec<PeerId>),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn blacklisted_peers<
    Backend,
    Membership,
    MembershipAdapter,
    MembershipStorage,
    ApiAdapter,
    RuntimeServiceId,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
) -> Response
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Backend::Message: MonitorMessageFactory,
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
    make_request_and_return_response!(da::blacklisted_peers::<
        Backend,
        Membership,
        MembershipAdapter,
        MembershipStorage,
        ApiAdapter,
        RuntimeServiceId,
    >(&handle))
}

#[utoipa::path(
    get,
    path = paths::NETWORK_INFO,
    responses(
        (status = 200, description = "Query the network information", body = nomos_network::backends::libp2p::Libp2pInfo),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn libp2p_info<RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
) -> Response
where
    RuntimeServiceId: Debug
        + Sync
        + Display
        + 'static
        + AsServiceId<
            nomos_network::NetworkService<
                nomos_network::backends::libp2p::Libp2p,
                RuntimeServiceId,
            >,
        >,
{
    make_request_and_return_response!(libp2p::libp2p_info::<RuntimeServiceId>(&handle))
}

#[utoipa::path(
    post,
    path = paths::STORAGE_BLOCK,
    responses(
        (status = 200, description = "Get the block by block id", body = HeaderId),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn block<HttpStorageAdapter, Tx, RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(id): Json<HeaderId>,
) -> Response
where
    Tx: Serialize + DeserializeOwned + Clone + Eq + Send + Sync + 'static,
    HttpStorageAdapter: StorageAdapter<RuntimeServiceId> + Send + Sync + 'static,
    RuntimeServiceId:
        AsServiceId<StorageService<RocksBackend, RuntimeServiceId>> + Debug + Sync + Display,
{
    let relay = match handle.relay().await {
        Ok(relay) => relay,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    make_request_and_return_response!(HttpStorageAdapter::get_block::<Tx>(relay, id))
}

#[utoipa::path(
    get,
    path = paths::DA_GET_SHARES_COMMITMENTS,
    responses(
        (status = 200, description = "Request the commitments for an specific `BlobId` that the node stores locally or otherwise requests from the subnetwork peers", body = DASharesCommitmentsRequest<DaShare>),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn da_get_commitments<
    DaBlobId,
    SamplingBackend,
    SamplingNetwork,
    SamplingStorage,
    RuntimeServiceId,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(blob_id): Json<DaBlobId>,
) -> Response
where
    DaBlobId: Serialize + for<'de> Deserialize<'de> + Send + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = DaBlobId>,
    SamplingNetwork: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + AsServiceId<
            DaSamplingService<SamplingBackend, SamplingNetwork, SamplingStorage, RuntimeServiceId>,
        >,
{
    make_request_and_return_response!(da::get_commitments::<
        SamplingBackend,
        SamplingNetwork,
        SamplingStorage,
        RuntimeServiceId,
    >(&handle, blob_id))
}

#[utoipa::path(
    get,
    path = paths::DA_GET_STORAGE_SHARES_COMMITMENTS,
    responses(
        (status = 200, description = "Request the commitments for an specific `BlobId` that the node stores locally", body = DASharesCommitmentsRequest<DaShare>),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn da_get_storage_commitments<
    DaStorageConverter,
    HttpStorageAdapter,
    DaShare,
    RuntimeServiceId,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(req): Json<DASharesCommitmentsRequest<DaShare>>,
) -> Response
where
    DaShare: Share,
    <DaShare as Share>::BlobId: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    <DaShare as Share>::SharesCommitments: Serialize + DeserializeOwned + Send + Sync + 'static,
    DaStorageConverter: DaConverter<DaStorageBackend, Share = DaShare> + Send + Sync + 'static,
    HttpStorageAdapter: StorageAdapter<RuntimeServiceId>,
    RuntimeServiceId: AsServiceId<StorageService<DaStorageBackend, RuntimeServiceId>>
        + Debug
        + Sync
        + Display
        + 'static,
{
    let relay = match handle.relay().await {
        Ok(relay) => relay,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    make_request_and_return_response!(HttpStorageAdapter::get_shared_commitments::<
        DaStorageConverter,
        DaShare,
    >(relay, req.blob_id))
}

#[utoipa::path(
    get,
    path = paths::DA_GET_LIGHT_SHARE,
    responses(
        (status = 200, description = "Get blob by blob id", body = DaSamplingRequest<DaShare>),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn da_get_light_share<DaStorageConverter, HttpStorageAdapter, DaShare, RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(request): Json<DaSamplingRequest<DaShare>>,
) -> Response
where
    DaShare: Share + Clone + Send + Sync + 'static,
    <DaShare as Share>::BlobId: Clone + DeserializeOwned + Send + Sync + 'static,
    <DaShare as Share>::ShareIndex: Clone + DeserializeOwned + Send + Sync + 'static,
    DaShare::LightShare: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    DaStorageConverter: DaConverter<RocksBackend, Share = DaShare> + Send + Sync + 'static,
    HttpStorageAdapter: StorageAdapter<RuntimeServiceId>,
    RuntimeServiceId: AsServiceId<StorageService<RocksBackend, RuntimeServiceId>>
        + Debug
        + Sync
        + Display
        + 'static,
{
    let relay = match handle.relay().await {
        Ok(relay) => relay,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    make_request_and_return_response!(HttpStorageAdapter::get_light_share::<
        DaStorageConverter,
        DaShare,
    >(relay, request.blob_id, request.share_idx))
}

#[utoipa::path(
    get,
    path = paths::DA_GET_LIGHT_SHARE,
    responses(
        (status = 200, description = "Request shares for a blob", body = GetSharesRequest<DaBlob>),
        (status = 500, description = "Internal server error", body = StreamBody),
    )
)]
pub async fn da_get_shares<DaStorageConverter, HttpStorageAdapter, DaShare, RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(request): Json<GetSharesRequest<DaShare>>,
) -> Response
where
    DaShare: Share + 'static,
    <DaShare as Share>::BlobId: Clone + Send + Sync + 'static,
    <DaShare as Share>::ShareIndex: Serialize + DeserializeOwned + Hash + Eq + Send + Sync,
    <DaShare as Share>::LightShare: Serialize + DeserializeOwned + Send + Sync + 'static,
    DaStorageConverter: DaConverter<RocksBackend, Share = DaShare> + Send + Sync + 'static,
    HttpStorageAdapter: StorageAdapter<RuntimeServiceId> + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + 'static
        + AsServiceId<StorageService<RocksBackend, RuntimeServiceId>>,
{
    let relay = match handle.relay().await {
        Ok(relay) => relay,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    match HttpStorageAdapter::get_shares::<DaStorageConverter, DaShare>(
        relay,
        request.blob_id,
        request.requested_shares,
        request.filter_shares,
        request.return_available,
    )
    .await
    {
        Ok(shares) => {
            let body = Body::from_stream(shares);
            match Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(body)
            {
                Ok(response) => response,
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[utoipa::path(
    get,
    path = paths::DA_BALANCER_STATS,
    responses(
        (status = 200, description = "Get balancer stats", body = String),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn balancer_stats<
    Backend,
    Membership,
    MembershipAdapter,
    MembershipStorage,
    ApiAdapter,
    RuntimeServiceId,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
) -> Response
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Backend::Message: BalancerMessageFactory,
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
    make_request_and_return_response!(da::balancer_stats::<
        Backend,
        Membership,
        MembershipAdapter,
        MembershipStorage,
        ApiAdapter,
        RuntimeServiceId,
    >(&handle))
}

#[utoipa::path(
    get,
    path = paths::DA_BALANCER_STATS,
    responses(
        (status = 200, description = "Get monitor stats", body = String),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn monitor_stats<
    Backend,
    Membership,
    MembershipAdapter,
    MembershipStorage,
    ApiAdapter,
    RuntimeServiceId,
>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
) -> Response
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Backend::Message: MonitorMessageFactory,
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
    make_request_and_return_response!(da::monitor_stats::<
        Backend,
        Membership,
        MembershipAdapter,
        MembershipStorage,
        ApiAdapter,
        RuntimeServiceId,
    >(&handle))
}

#[utoipa::path(
    post,
    path = paths::MEMPOOL_ADD_TX,
    responses(
        (status = 200, description = "Add transaction to the mempool"),
        (status = 500, description = "Internal server error", body = String),
    )
)]
pub async fn add_tx<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>(
    State(handle): State<OverwatchHandle<RuntimeServiceId>>,
    Json(tx): Json<Tx>,
) -> Response
where
    Tx: AuthenticatedMantleTx
        + Clone
        + Debug
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <Tx as Transaction>::Hash:
        Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    RuntimeServiceId: Debug
        + Sync
        + Send
        + Display
        + 'static
        + AsServiceId<
            TxMempoolService<
                MempoolNetworkAdapter<Tx, <Tx as Transaction>::Hash, RuntimeServiceId>,
                SamplingNetworkAdapter,
                SamplingStorage,
                MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
                RuntimeServiceId,
            >,
        >,
{
    make_request_and_return_response!(mempool::add_tx::<
        Libp2pNetworkBackend,
        MempoolNetworkAdapter<Tx, <Tx as Transaction>::Hash, RuntimeServiceId>,
        SamplingNetworkAdapter,
        SamplingStorage,
        Tx,
        <Tx as Transaction>::Hash,
        RuntimeServiceId,
    >(&handle, tx, Transaction::hash))
}
