use std::{fmt::Debug, hash::Hash};

use axum::{
    extract::{Query, State},
    http::HeaderValue,
    response::Response,
    routing, Json, Router, Server,
};
use hyper::header::{CONTENT_TYPE, USER_AGENT};
use overwatch_rs::overwatch::handle::OverwatchHandle;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use consensus_engine::BlockId;
use full_replication::{Blob, Certificate};
use nomos_core::{da::blob, tx::Transaction};
use nomos_mempool::{network::adapters::libp2p::Libp2pAdapter, openapi::Status, MempoolMetrics};
use nomos_network::backends::libp2p::Libp2p;
use nomos_storage::backends::StorageSerde;

use crate::{
    http::{cl, consensus, da, libp2p, mempool, storage},
    Backend,
};

/// Configuration for the Http Server
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct AxumBackendSettings {
    /// Socket where the server will be listening on for incoming requests.
    pub address: std::net::SocketAddr,
    /// Allowed origins for this server deployment requests.
    pub cors_origins: Vec<String>,
}

pub struct AxumBackend<T, S, const SIZE: usize> {
    settings: AxumBackendSettings,
    _tx: core::marker::PhantomData<T>,
    _storage_serde: core::marker::PhantomData<S>,
}

#[derive(OpenApi)]
#[openapi(
    paths(
        da_metrics,
        da_status,
    ),
    components(
        schemas(Status, MempoolMetrics)
    ),
    tags(
        (name = "da", description = "data availibility related APIs")
    )
)]
struct ApiDoc;

#[async_trait::async_trait]
impl<T, S, const SIZE: usize> Backend for AxumBackend<T, S, SIZE>
where
    T: Transaction
        + Clone
        + Debug
        + Eq
        + Hash
        + Serialize
        + for<'de> Deserialize<'de>
        + Send
        + Sync
        + 'static,
    <T as nomos_core::tx::Transaction>::Hash:
        Serialize + for<'de> Deserialize<'de> + std::cmp::Ord + Debug + Send + Sync + 'static,
    S: StorageSerde + Send + Sync + 'static,
{
    type Error = hyper::Error;
    type Settings = AxumBackendSettings;

    async fn new(settings: Self::Settings) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        Ok(Self {
            settings,
            _tx: core::marker::PhantomData,
            _storage_serde: core::marker::PhantomData,
        })
    }

    async fn serve(self, handle: OverwatchHandle) -> Result<(), Self::Error> {
        let mut builder = CorsLayer::new();
        if self.settings.cors_origins.is_empty() {
            builder = builder.allow_origin(Any);
        }

        for origin in &self.settings.cors_origins {
            builder = builder.allow_origin(
                origin
                    .as_str()
                    .parse::<HeaderValue>()
                    .expect("fail to parse origin"),
            );
        }

        let app = Router::new()
            .layer(
                builder
                    .allow_headers([CONTENT_TYPE, USER_AGENT])
                    .allow_methods(Any),
            )
            .layer(TraceLayer::new_for_http())
            .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
            .route("/da/metrics", routing::get(da_metrics))
            .route("/da/status", routing::post(da_status))
            .route("/da/blobs", routing::post(da_blobs))
            .route("/cl/metrics", routing::get(cl_metrics::<T>))
            .route("/cl/status", routing::post(cl_status::<T>))
            .route("/carnot/info", routing::get(carnot_info::<T, S, SIZE>))
            .route("/carnot/blocks", routing::get(carnot_blocks::<T, S, SIZE>))
            .route("/network/info", routing::get(libp2p_info))
            .route("/storage/block", routing::post(block::<S, T>))
            .route("/mempool/add/tx", routing::post(add_tx::<T>))
            .route("/mempool/add/cert", routing::post(add_cert))
            .with_state(handle);

        Server::bind(&self.settings.address)
            .serve(app.into_make_service())
            .await
    }
}

macro_rules! make_request_and_return_response {
    ($cond:expr) => {{
        match $cond.await {
            ::std::result::Result::Ok(val) => ::axum::response::IntoResponse::into_response((
                ::hyper::StatusCode::OK,
                ::axum::Json(val),
            )),
            ::std::result::Result::Err(e) => ::axum::response::IntoResponse::into_response((
                ::hyper::StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
            )),
        }
    }};
}

#[utoipa::path(
    get,
    path = "/da/metrics",
    responses(
        (status = 200, description = "Get the mempool metrics of the da service", body = MempoolMetrics),
        (status = 500, description = "Internal server error", body = String),
    )
)]
async fn da_metrics(State(handle): State<OverwatchHandle>) -> Response {
    make_request_and_return_response!(da::da_mempool_metrics(&handle))
}

#[utoipa::path(
    post,
    path = "/da/status",
    responses(
        (status = 200, description = "Query the mempool status of the da service", body = Vec<<Blob as blob::Blob>::Hash>),
        (status = 500, description = "Internal server error", body = String),
    )
)]
async fn da_status(
    State(handle): State<OverwatchHandle>,
    Json(items): Json<Vec<<Blob as blob::Blob>::Hash>>,
) -> Response {
    make_request_and_return_response!(da::da_mempool_status(&handle, items))
}

#[utoipa::path(
    post,
    path = "/da/blobs",
    responses(
        (status = 200, description = "Get pending blobs", body = Vec<Blob>),
        (status = 500, description = "Internal server error", body = String),
    )
)]
async fn da_blobs(
    State(handle): State<OverwatchHandle>,
    Json(items): Json<Vec<<Blob as blob::Blob>::Hash>>,
) -> Response {
    make_request_and_return_response!(da::da_blobs(&handle, items))
}

#[utoipa::path(
    get,
    path = "/cl/metrics",
    responses(
        (status = 200, description = "Get the mempool metrics of the cl service", body = MempoolMetrics),
        (status = 500, description = "Internal server error", body = String),
    )
)]
async fn cl_metrics<T>(State(handle): State<OverwatchHandle>) -> Response
where
    T: Transaction
        + Clone
        + Debug
        + Hash
        + Serialize
        + for<'de> Deserialize<'de>
        + Send
        + Sync
        + 'static,
    <T as nomos_core::tx::Transaction>::Hash: std::cmp::Ord + Debug + Send + Sync + 'static,
{
    make_request_and_return_response!(cl::cl_mempool_metrics::<T>(&handle))
}

#[utoipa::path(
    post,
    path = "/cl/status",
    responses(
        (status = 200, description = "Query the mempool status of the cl service", body = Vec<<T as Transaction>::Hash>),
        (status = 500, description = "Internal server error", body = String),
    )
)]
async fn cl_status<T>(
    State(handle): State<OverwatchHandle>,
    Json(items): Json<Vec<<T as Transaction>::Hash>>,
) -> Response
where
    T: Transaction + Clone + Debug + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
    <T as nomos_core::tx::Transaction>::Hash:
        Serialize + DeserializeOwned + std::cmp::Ord + Debug + Send + Sync + 'static,
{
    make_request_and_return_response!(cl::cl_mempool_status::<T>(&handle, items))
}

#[utoipa::path(
    get,
    path = "/carnot/info",
    responses(
        (status = 200, description = "Query the carnot information", body = nomos_consensus::CarnotInfo),
        (status = 500, description = "Internal server error", body = String),
    )
)]
async fn carnot_info<Tx, SS, const SIZE: usize>(State(handle): State<OverwatchHandle>) -> Response
where
    Tx: Transaction + Clone + Debug + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
    <Tx as Transaction>::Hash: std::cmp::Ord + Debug + Send + Sync + 'static,
    SS: StorageSerde + Send + Sync + 'static,
{
    make_request_and_return_response!(consensus::carnot_info::<Tx, SS, SIZE>(&handle))
}

#[derive(Deserialize)]
struct QueryParams {
    from: Option<BlockId>,
    to: Option<BlockId>,
}

#[utoipa::path(
    get,
    path = "/carnot/blocks",
    responses(
        (status = 200, description = "Query the block information", body = Vec<consensus_engine::Block>),
        (status = 500, description = "Internal server error", body = String),
    )
)]
async fn carnot_blocks<Tx, SS, const SIZE: usize>(
    State(store): State<OverwatchHandle>,
    Query(query): Query<QueryParams>,
) -> Response
where
    Tx: Transaction + Clone + Debug + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
    <Tx as Transaction>::Hash: std::cmp::Ord + Debug + Send + Sync + 'static,
    SS: StorageSerde + Send + Sync + 'static,
{
    let QueryParams { from, to } = query;
    make_request_and_return_response!(consensus::carnot_blocks::<Tx, SS, SIZE>(&store, from, to))
}

#[utoipa::path(
    get,
    path = "/network/info",
    responses(
        (status = 200, description = "Query the network information", body = nomos_network::backends::libp2p::Libp2pInfo),
        (status = 500, description = "Internal server error", body = String),
    )
)]
async fn libp2p_info(State(handle): State<OverwatchHandle>) -> Response {
    make_request_and_return_response!(libp2p::libp2p_info(&handle))
}

#[utoipa::path(
    get,
    path = "/storage/block",
    responses(
        (status = 200, description = "Get the block by block id", body = Block<Tx, full_replication::Certificate>),
        (status = 500, description = "Internal server error", body = String),
    )
)]
async fn block<S, Tx>(State(handle): State<OverwatchHandle>, Json(id): Json<BlockId>) -> Response
where
    Tx: serde::Serialize + serde::de::DeserializeOwned + Clone + Eq + core::hash::Hash,
    S: StorageSerde + Send + Sync + 'static,
{
    make_request_and_return_response!(storage::block_req::<S, Tx>(&handle, id))
}

#[utoipa::path(
    post,
    path = "/mempool/add/tx",
    responses(
        (status = 200, description = "Add transaction to the mempool"),
        (status = 500, description = "Internal server error", body = String),
    )
)]
async fn add_tx<Tx>(State(handle): State<OverwatchHandle>, Json(tx): Json<Tx>) -> Response
where
    Tx: Transaction + Clone + Debug + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
    <Tx as Transaction>::Hash: std::cmp::Ord + Debug + Send + Sync + 'static,
{
    make_request_and_return_response!(mempool::add::<
        Libp2p,
        Libp2pAdapter<Tx, <Tx as Transaction>::Hash>,
        nomos_mempool::Transaction,
        Tx,
        <Tx as Transaction>::Hash,
    >(&handle, tx, Transaction::hash))
}

#[utoipa::path(
    post,
    path = "/mempool/add/cert",
    responses(
        (status = 200, description = "Add certificate to the mempool"),
        (status = 500, description = "Internal server error", body = String),
    )
)]
async fn add_cert(
    State(handle): State<OverwatchHandle>,
    Json(cert): Json<Certificate>,
) -> Response {
    make_request_and_return_response!(mempool::add::<
        Libp2p,
        Libp2pAdapter<Certificate, <Blob as blob::Blob>::Hash>,
        nomos_mempool::Certificate,
        Certificate,
        <Blob as blob::Blob>::Hash,
    >(
        &handle,
        cert,
        nomos_core::da::certificate::Certificate::hash
    ))
}
