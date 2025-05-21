use std::{
    error::Error,
    fmt::{Debug, Display},
    hash::Hash,
};

use axum::{http::HeaderValue, routing, Router, Server};
use hyper::header::{CONTENT_TYPE, USER_AGENT};
use nomos_api::{
    http::{
        consensus::Cryptarchia,
        da::{DaIndexer, DaVerifier},
        storage,
    },
    Backend,
};
use nomos_core::{
    da::{
        blob::{info::DispersedBlobInfo, metadata::Metadata, LightShare, Share},
        DaVerifier as CoreDaVerifier,
    },
    header::HeaderId,
    tx::Transaction,
};
use nomos_da_network_core::SubnetworkId;
use nomos_da_network_service::backends::libp2p::validator::DaNetworkValidatorBackend;
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_da_verifier::backend::VerifierBackend;
use nomos_http_api_common::paths;
use nomos_libp2p::PeerId;
use nomos_mempool::{
    backend::mockpool::MockPool, tx::service::openapi::Status, DaMempoolService, MempoolMetrics,
    TxMempoolService,
};
use nomos_storage::{
    api::da::StorageDaApi,
    backends::{rocksdb::RocksBackend, StorageSerde},
    StorageService,
};
use overwatch::{overwatch::handle::OverwatchHandle, services::AsServiceId};
use rand::{RngCore, SeedableRng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use subnetworks_assignations::MembershipHandler;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use super::handlers::{
    add_blob_info, add_share, add_tx, balancer_stats, blacklisted_peers, block, block_peer,
    cl_metrics, cl_status, cryptarchia_headers, cryptarchia_info, da_get_commitments,
    da_get_light_share, da_get_shares, get_range, libp2p_info, monitor_stats, unblock_peer,
};

pub(crate) type DaStorageBackend<SerdeOp> = RocksBackend<SerdeOp>;
type DaService<DaStorageSerializer, RuntimeServiceId> =
    StorageService<DaStorageBackend<DaStorageSerializer>, RuntimeServiceId>;

/// Configuration for the Http Server
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct AxumBackendSettings {
    /// Socket where the server will be listening on for incoming requests.
    pub address: std::net::SocketAddr,
    /// Allowed origins for this server deployment requests.
    pub cors_origins: Vec<String>,
}

pub struct AxumBackend<
    DaAttestation,
    DaShare,
    DaBlobInfo,
    Membership,
    DaVerifiedBlobInfo,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
    Tx,
    DaStorageSerializer,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingRng,
    SamplingStorage,
    TimeBackend,
    ApiAdapter,
    HttpStorageAdapter,
    const SIZE: usize,
> {
    settings: AxumBackendSettings,
    _attestation: core::marker::PhantomData<DaAttestation>,
    _share: core::marker::PhantomData<DaShare>,
    _certificate: core::marker::PhantomData<DaBlobInfo>,
    _membership: core::marker::PhantomData<Membership>,
    _vid: core::marker::PhantomData<DaVerifiedBlobInfo>,
    _verifier_backend: core::marker::PhantomData<DaVerifierBackend>,
    _verifier_network: core::marker::PhantomData<DaVerifierNetwork>,
    _verifier_storage: core::marker::PhantomData<DaVerifierStorage>,
    _tx: core::marker::PhantomData<Tx>,
    _storage_serde: core::marker::PhantomData<DaStorageSerializer>,
    _sampling_backend: core::marker::PhantomData<SamplingBackend>,
    _sampling_network_adapter: core::marker::PhantomData<SamplingNetworkAdapter>,
    _sampling_rng: core::marker::PhantomData<SamplingRng>,
    _sampling_storage: core::marker::PhantomData<SamplingStorage>,
    _time_backend: core::marker::PhantomData<TimeBackend>,
    _api_adapter: core::marker::PhantomData<ApiAdapter>,
    _storage_adapter: core::marker::PhantomData<HttpStorageAdapter>,
}

#[derive(OpenApi)]
#[openapi(
    paths(
    ),
    components(
        schemas(Status<HeaderId>, MempoolMetrics)
    ),
    tags(
        (name = "da", description = "data availibility related APIs")
    )
)]
struct ApiDoc;

#[async_trait::async_trait]
impl<
        DaAttestation,
        DaShare,
        DaBlobInfo,
        Membership,
        DaVerifiedBlobInfo,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        Tx,
        DaStorageSerializer,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        TimeBackend,
        ApiAdapter,
        StorageAdapter,
        const SIZE: usize,
        RuntimeServiceId,
    > Backend<RuntimeServiceId>
    for AxumBackend<
        DaAttestation,
        DaShare,
        DaBlobInfo,
        Membership,
        DaVerifiedBlobInfo,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        Tx,
        DaStorageSerializer,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        TimeBackend,
        ApiAdapter,
        StorageAdapter,
        SIZE,
    >
where
    DaAttestation: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    DaShare: Share + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    <DaShare as Share>::BlobId:
        AsRef<[u8]> + Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    <DaShare as Share>::ShareIndex: AsRef<[u8]> + DeserializeOwned + Send + Sync + 'static,
    DaShare::LightShare: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    DaShare::SharesCommitments: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    DaBlobInfo: DispersedBlobInfo<BlobId = [u8; 32]>
        + Clone
        + Debug
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <DaBlobInfo as DispersedBlobInfo>::BlobId: Clone + Send + Sync,
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    DaVerifiedBlobInfo: DispersedBlobInfo<BlobId = [u8; 32]>
        + From<DaBlobInfo>
        + Eq
        + Debug
        + Metadata
        + Hash
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <DaVerifiedBlobInfo as DispersedBlobInfo>::BlobId: Debug + Clone + Ord + Hash,
    <DaVerifiedBlobInfo as Metadata>::AppId:
        AsRef<[u8]> + Clone + Serialize + DeserializeOwned + Send + Sync,
    <DaVerifiedBlobInfo as Metadata>::Index:
        AsRef<[u8]> + Clone + Serialize + DeserializeOwned + PartialOrd + Send + Sync,
    DaVerifierBackend: VerifierBackend + CoreDaVerifier<DaShare = DaShare> + Send + Sync + 'static,
    <DaVerifierBackend as VerifierBackend>::Settings: Clone,
    <DaVerifierBackend as CoreDaVerifier>::Error: Error,
    DaVerifierNetwork:
        nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
    DaVerifierStorage:
        nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync + 'static,
    Tx: Transaction
        + Clone
        + Debug
        + Eq
        + Hash
        + Serialize
        + for<'de> Deserialize<'de>
        + Send
        + Sync
        + 'static,
    <Tx as nomos_core::tx::Transaction>::Hash:
        Serialize + for<'de> Deserialize<'de> + std::cmp::Ord + Debug + Send + Sync + 'static,
    DaStorageSerializer: StorageSerde + Send + Sync + 'static,
    <DaStorageSerializer as StorageSerde>::Error: Send + Sync,
    SamplingRng: SeedableRng + RngCore + Send + 'static,
    SamplingBackend: DaSamplingServiceBackend<
            SamplingRng,
            BlobId = <DaVerifiedBlobInfo as DispersedBlobInfo>::BlobId,
        > + Send
        + 'static,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    DaShare::LightShare: LightShare<ShareIndex = <DaShare as Share>::ShareIndex>
        + Serialize
        + DeserializeOwned
        + Clone
        + Send
        + Sync
        + 'static,
    <DaShare as Share>::ShareIndex:
        AsRef<[u8]> + Serialize + DeserializeOwned + Hash + Eq + Send + Sync + 'static,
    DaShare::LightShare: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    DaShare::SharesCommitments: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + 'static,
    SamplingStorage:
        nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + 'static,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend + Send + 'static,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync + 'static,
    StorageAdapter:
        storage::StorageAdapter<DaStorageSerializer, RuntimeServiceId> + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Send
        + Display
        + Clone
        + 'static
        + AsServiceId<
            Cryptarchia<
                Tx,
                DaStorageSerializer,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingRng,
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
                TimeBackend,
                ApiAdapter,
                RuntimeServiceId,
                SIZE,
            >,
        >
        + AsServiceId<
            DaVerifier<
                DaShare,
                DaVerifierNetwork,
                DaVerifierBackend,
                DaStorageSerializer,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<
            DaIndexer<
                Tx,
                DaBlobInfo,
                DaVerifiedBlobInfo,
                DaStorageSerializer,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingRng,
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
                TimeBackend,
                ApiAdapter,
                RuntimeServiceId,
                SIZE,
            >,
        >
        + AsServiceId<
            nomos_da_network_service::NetworkService<
                DaNetworkValidatorBackend<Membership>,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<
            nomos_network::NetworkService<
                nomos_network::backends::libp2p::Libp2p,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<DaService<DaStorageSerializer, RuntimeServiceId>>
        + AsServiceId<
            TxMempoolService<
                nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
                    Tx,
                    <Tx as Transaction>::Hash,
                    RuntimeServiceId,
                >,
                MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<
            DaMempoolService<
                nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
                    DaVerifiedBlobInfo,
                    DaVerifiedBlobInfo::BlobId,
                    RuntimeServiceId,
                >,
                MockPool<HeaderId, DaVerifiedBlobInfo, DaVerifiedBlobInfo::BlobId>,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingRng,
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
                ApiAdapter,
                RuntimeServiceId,
            >,
        >,
    // Service and storage layer types conversions
    <DaStorageBackend<DaStorageSerializer> as StorageDaApi>::BlobId: From<DaShare::BlobId>,
    <DaStorageBackend<DaStorageSerializer> as StorageDaApi>::ShareIndex:
        Into<DaShare::ShareIndex> + From<DaShare::ShareIndex>,
    <DaStorageBackend<DaStorageSerializer> as StorageDaApi>::Share: TryInto<DaShare::LightShare>,
{
    type Error = hyper::Error;
    type Settings = AxumBackendSettings;

    async fn new(settings: Self::Settings) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        Ok(Self {
            settings,
            _attestation: core::marker::PhantomData,
            _share: core::marker::PhantomData,
            _certificate: core::marker::PhantomData,
            _membership: core::marker::PhantomData,
            _vid: core::marker::PhantomData,
            _verifier_backend: core::marker::PhantomData,
            _verifier_network: core::marker::PhantomData,
            _verifier_storage: core::marker::PhantomData,
            _tx: core::marker::PhantomData,
            _storage_serde: core::marker::PhantomData,
            _sampling_backend: core::marker::PhantomData,
            _sampling_network_adapter: core::marker::PhantomData,
            _sampling_rng: core::marker::PhantomData,
            _sampling_storage: core::marker::PhantomData,
            _time_backend: core::marker::PhantomData,
            _api_adapter: core::marker::PhantomData,
            _storage_adapter: core::marker::PhantomData,
        })
    }

    #[expect(clippy::too_many_lines, reason = "TODO: Address this at some point.")]
    async fn serve(self, handle: OverwatchHandle<RuntimeServiceId>) -> Result<(), Self::Error> {
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
            .route(
                paths::CL_METRICS,
                routing::get(cl_metrics::<Tx, RuntimeServiceId>),
            )
            .route(
                paths::CL_STATUS,
                routing::post(cl_status::<Tx, RuntimeServiceId>),
            )
            .route(
                paths::CRYPTARCHIA_INFO,
                routing::get(
                    cryptarchia_info::<
                        Tx,
                        DaStorageSerializer,
                        SamplingBackend,
                        SamplingNetworkAdapter,
                        SamplingRng,
                        SamplingStorage,
                        DaVerifierBackend,
                        DaVerifierNetwork,
                        DaVerifierStorage,
                        TimeBackend,
                        ApiAdapter,
                        RuntimeServiceId,
                        SIZE,
                    >,
                ),
            )
            .route(
                paths::CRYPTARCHIA_HEADERS,
                routing::get(
                    cryptarchia_headers::<
                        Tx,
                        DaStorageSerializer,
                        SamplingBackend,
                        SamplingNetworkAdapter,
                        SamplingRng,
                        SamplingStorage,
                        DaVerifierBackend,
                        DaVerifierNetwork,
                        DaVerifierStorage,
                        TimeBackend,
                        ApiAdapter,
                        RuntimeServiceId,
                        SIZE,
                    >,
                ),
            )
            .route(
                paths::DA_ADD_SHARE,
                routing::post(
                    add_share::<
                        DaAttestation,
                        DaShare,
                        DaVerifierNetwork,
                        DaVerifierBackend,
                        DaStorageSerializer,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_GET_RANGE,
                routing::post(
                    get_range::<
                        Tx,
                        DaBlobInfo,
                        DaVerifiedBlobInfo,
                        DaStorageSerializer,
                        SamplingBackend,
                        SamplingNetworkAdapter,
                        SamplingRng,
                        SamplingStorage,
                        DaVerifierBackend,
                        DaVerifierNetwork,
                        DaVerifierStorage,
                        TimeBackend,
                        ApiAdapter,
                        RuntimeServiceId,
                        SIZE,
                    >,
                ),
            )
            .route(
                paths::DA_BLOCK_PEER,
                routing::post(
                    block_peer::<DaNetworkValidatorBackend<Membership>, RuntimeServiceId>,
                ),
            )
            .route(
                paths::DA_UNBLOCK_PEER,
                routing::post(
                    unblock_peer::<DaNetworkValidatorBackend<Membership>, RuntimeServiceId>,
                ),
            )
            .route(
                paths::DA_BLACKLISTED_PEERS,
                routing::get(
                    blacklisted_peers::<DaNetworkValidatorBackend<Membership>, RuntimeServiceId>,
                ),
            )
            .route(
                paths::NETWORK_INFO,
                routing::get(libp2p_info::<RuntimeServiceId>),
            )
            .route(
                paths::STORAGE_BLOCK,
                routing::post(block::<DaStorageSerializer, StorageAdapter, Tx, RuntimeServiceId>),
            )
            .route(
                paths::MEMPOOL_ADD_TX,
                routing::post(add_tx::<Tx, RuntimeServiceId>),
            )
            .route(
                paths::MEMPOOL_ADD_BLOB_INFO,
                routing::post(
                    add_blob_info::<
                        DaVerifiedBlobInfo,
                        SamplingBackend,
                        SamplingNetworkAdapter,
                        SamplingRng,
                        SamplingStorage,
                        DaVerifierBackend,
                        DaVerifierNetwork,
                        DaVerifierStorage,
                        ApiAdapter,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_GET_SHARES_COMMITMENTS,
                routing::get(
                    da_get_commitments::<
                        DaStorageSerializer,
                        StorageAdapter,
                        DaShare,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_GET_LIGHT_SHARE,
                routing::get(
                    da_get_light_share::<
                        DaStorageSerializer,
                        StorageAdapter,
                        DaShare,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_GET_SHARES,
                routing::get(
                    da_get_shares::<DaStorageSerializer, StorageAdapter, DaShare, RuntimeServiceId>,
                ),
            )
            .route(
                paths::DA_BALANCER_STATS,
                routing::get(
                    balancer_stats::<DaNetworkValidatorBackend<Membership>, RuntimeServiceId>,
                ),
            )
            .route(
                paths::DA_MONITOR_STATS,
                routing::get(
                    monitor_stats::<DaNetworkValidatorBackend<Membership>, RuntimeServiceId>,
                ),
            )
            .with_state(handle);

        Server::bind(&self.settings.address)
            .serve(app.into_make_service())
            .await
    }
}
