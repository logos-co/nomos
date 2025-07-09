use std::{
    error::Error,
    fmt::{Debug, Display},
    hash::Hash,
    time::Duration,
};

use axum::{http::HeaderValue, routing, Router, Server};
use hyper::header::{CONTENT_TYPE, USER_AGENT};
use nomos_api::{
    http::{
        consensus::Cryptarchia,
        da::{DaDispersal, DaIndexer, DaVerifier},
        storage,
    },
    Backend,
};
use nomos_core::{
    da::{
        blob::{info::DispersedBlobInfo, metadata, LightShare, Share},
        DaVerifier as CoreDaVerifier,
    },
    header::HeaderId,
    mantle::Transaction,
};
use nomos_da_dispersal::adapters::mempool::DaMempoolAdapter;
use nomos_da_network_core::SubnetworkId;
use nomos_da_network_service::{
    backends::libp2p::executor::DaNetworkExecutorBackend, membership::MembershipAdapter,
    storage::MembershipStorageAdapter,
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_da_verifier::backend::VerifierBackend;
use nomos_http_api_common::paths;
use nomos_libp2p::PeerId;
use nomos_mempool::{
    backend::mockpool::MockPool, tx::service::openapi::Status, DaMempoolService, MempoolMetrics,
    TxMempoolService,
};
use nomos_node::{
    api::handlers::{
        add_blob_info, add_share, add_tx, balancer_stats, blacklisted_peers, block, block_peer,
        cl_metrics, cl_status, cryptarchia_headers, cryptarchia_info, da_get_commitments,
        da_get_light_share, da_get_shares, get_range, libp2p_info, monitor_stats, unblock_peer,
    },
    RocksBackend,
};
use nomos_storage::{api::da, backends::StorageSerde, StorageService};
use overwatch::{overwatch::handle::OverwatchHandle, services::AsServiceId, DynError};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use subnetworks_assignations::MembershipHandler;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use super::handlers::disperse_data;

type DaStorageBackend<SerdeOp> = RocksBackend<SerdeOp>;
type DaStorageService<DaStorageSerializer, RuntimeServiceId> =
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
    Memebership,
    DaMembershipAdapter,
    DaMembershipStorage,
    DaVerifiedBlobInfo,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
    Tx,
    DaStorageSerializer,
    DaStorageConverter,
    DispersalBackend,
    DispersalNetworkAdapter,
    DispersalMempoolAdapter,
    Metadata,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    ApiAdapter,
    HttpStorageAdapter,
    const SIZE: usize,
> {
    settings: AxumBackendSettings,
    #[expect(clippy::allow_attributes_without_reason)]
    #[expect(clippy::type_complexity)]
    _phantom: core::marker::PhantomData<(
        DaAttestation,
        DaShare,
        DaBlobInfo,
        Memebership,
        DaMembershipAdapter,
        DaMembershipStorage,
        DaVerifiedBlobInfo,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        Tx,
        DaStorageSerializer,
        DaStorageConverter,
        DispersalBackend,
        DispersalNetworkAdapter,
        DispersalMempoolAdapter,
        Metadata,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        ApiAdapter,
        HttpStorageAdapter,
    )>,
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
        DaMembershipAdapter,
        DaMembershipStorage,
        DaVerifiedBlobInfo,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        Tx,
        DaStorageSerializer,
        DaStorageConverter,
        DispersalBackend,
        DispersalNetworkAdapter,
        DispersalMempoolAdapter,
        Metadata,
        SamplingBackend,
        SamplingNetworkAdapter,
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
        DaMembershipAdapter,
        DaMembershipStorage,
        DaVerifiedBlobInfo,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        Tx,
        DaStorageSerializer,
        DaStorageConverter,
        DispersalBackend,
        DispersalNetworkAdapter,
        DispersalMempoolAdapter,
        Metadata,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        ApiAdapter,
        StorageAdapter,
        SIZE,
    >
where
    DaAttestation: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    DaShare: Share + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    <DaShare as Share>::BlobId: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    <DaShare as Share>::ShareIndex:
        Clone + Serialize + DeserializeOwned + Hash + Eq + Send + Sync + 'static,
    <DaShare as Share>::SharesCommitments:
        Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    <DaShare as Share>::LightShare: LightShare<ShareIndex = <DaShare as Share>::ShareIndex>
        + Serialize
        + DeserializeOwned
        + Clone
        + Send
        + Sync
        + 'static,
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
        + metadata::Metadata
        + Hash
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <DaVerifiedBlobInfo as DispersedBlobInfo>::BlobId: Debug + Clone + Ord + Hash,
    <DaVerifiedBlobInfo as metadata::Metadata>::AppId:
        AsRef<[u8]> + Clone + Serialize + DeserializeOwned + Send + Sync,
    <DaVerifiedBlobInfo as metadata::Metadata>::Index:
        AsRef<[u8]> + Clone + Serialize + DeserializeOwned + PartialOrd + Send + Sync,
    DaVerifierBackend: VerifierBackend + CoreDaVerifier<DaShare = DaShare> + Send + Sync + 'static,
    <DaVerifierBackend as VerifierBackend>::Settings: Clone,
    <DaVerifierBackend as CoreDaVerifier>::Error: Error,
    DaVerifierNetwork:
        nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
    DaVerifierNetwork::Settings: Clone,
    DaVerifierStorage:
        nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync + 'static,
    DaVerifierStorage::Settings: Clone,
    DaMembershipAdapter: MembershipAdapter + Send + Sync + 'static,
    DaMembershipStorage: MembershipStorageAdapter<PeerId, SubnetworkId> + Send + Sync + 'static,
    Tx: Transaction
        + Clone
        + Debug
        + Eq
        + Serialize
        + for<'de> Deserialize<'de>
        + Send
        + Sync
        + 'static,
    <Tx as Transaction>::Hash:
        Serialize + for<'de> Deserialize<'de> + Ord + Debug + Send + Sync + 'static,
    DaStorageSerializer: StorageSerde + Send + Sync + 'static,
    <DaStorageSerializer as StorageSerde>::Error: Send + Sync,
    DaStorageConverter: da::DaConverter<DaStorageBackend<DaStorageSerializer>, Share = DaShare>
        + Send
        + Sync
        + 'static,
    DispersalBackend: nomos_da_dispersal::backend::DispersalBackend<
            NetworkAdapter = DispersalNetworkAdapter,
            MempoolAdapter = DispersalMempoolAdapter,
            Metadata = Metadata,
        > + Send
        + Sync
        + 'static,
    DispersalBackend::BlobId: Serialize,
    DispersalBackend::Settings: Clone + Send + Sync,
    DispersalNetworkAdapter: nomos_da_dispersal::adapters::network::DispersalNetworkAdapter<
            SubnetworkId = Membership::NetworkId,
        > + Send
        + 'static,
    DispersalMempoolAdapter: DaMempoolAdapter + Send + 'static,
    Metadata: DeserializeOwned + metadata::Metadata + Debug + Send + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = <DaVerifiedBlobInfo as DispersedBlobInfo>::BlobId>
        + Send
        + 'static,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    <DaShare as Share>::LightShare: LightShare<ShareIndex = <DaShare as Share>::ShareIndex>
        + Serialize
        + DeserializeOwned
        + Clone
        + Send
        + Sync
        + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + 'static,
    SamplingStorage:
        nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + 'static,
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
                DaStorageConverter,
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
                DaNetworkExecutorBackend<Membership>,
                Membership,
                DaMembershipAdapter,
                DaMembershipStorage,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<
            nomos_network::NetworkService<
                nomos_network::backends::libp2p::Libp2p,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<DaStorageService<DaStorageSerializer, RuntimeServiceId>>
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
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
                ApiAdapter,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<
            DaDispersal<
                DispersalBackend,
                DispersalNetworkAdapter,
                DispersalMempoolAdapter,
                Membership,
                Metadata,
                RuntimeServiceId,
            >,
        >,
{
    type Error = hyper::Error;
    type Settings = AxumBackendSettings;

    async fn new(settings: Self::Settings) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        Ok(Self {
            settings,
            _phantom: core::marker::PhantomData,
        })
    }

    async fn wait_until_ready(
        &mut self,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    ) -> Result<(), DynError> {
        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            Cryptarchia<_, _, _, _, _, _, _, _, _, _, _, SIZE>,
            DaVerifier<_, _, _, _, _, _>,
            DaIndexer<_, _, _, _, _, _, _, _, _, _, _, _, _, SIZE>,
            nomos_da_network_service::NetworkService<_, _, _, _,_>,
            nomos_network::NetworkService<_, _>,
            DaStorageService<_, _>,
            TxMempoolService<_, _, _>,
            DaMempoolService<_, _, _, _, _, _, _, _, _, _>,
            DaDispersal<_, _, _, _, _, _>
        )
        .await
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
                        DaStorageConverter,
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
                    block_peer::<
                        DaNetworkExecutorBackend<Membership>,
                        Membership,
                        DaMembershipAdapter,
                        DaMembershipStorage,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_UNBLOCK_PEER,
                routing::post(
                    unblock_peer::<
                        DaNetworkExecutorBackend<Membership>,
                        Membership,
                        DaMembershipAdapter,
                        DaMembershipStorage,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_BLACKLISTED_PEERS,
                routing::get(
                    blacklisted_peers::<
                        DaNetworkExecutorBackend<Membership>,
                        Membership,
                        DaMembershipAdapter,
                        DaMembershipStorage,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(paths::NETWORK_INFO, routing::get(libp2p_info))
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
                paths::DISPERSE_DATA,
                routing::post(
                    disperse_data::<
                        DispersalBackend,
                        DispersalNetworkAdapter,
                        DispersalMempoolAdapter,
                        Membership,
                        Metadata,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_GET_SHARES_COMMITMENTS,
                routing::get(
                    da_get_commitments::<
                        DaStorageSerializer,
                        DaStorageConverter,
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
                        DaStorageConverter,
                        StorageAdapter,
                        DaShare,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_GET_SHARES,
                routing::get(
                    da_get_shares::<
                        DaStorageSerializer,
                        DaStorageConverter,
                        StorageAdapter,
                        DaShare,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_BALANCER_STATS,
                routing::get(
                    balancer_stats::<
                        DaNetworkExecutorBackend<Membership>,
                        Membership,
                        DaMembershipAdapter,
                        DaMembershipStorage,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_MONITOR_STATS,
                routing::get(
                    monitor_stats::<
                        DaNetworkExecutorBackend<Membership>,
                        Membership,
                        DaMembershipAdapter,
                        DaMembershipStorage,
                        RuntimeServiceId,
                    >,
                ),
            )
            .with_state(handle);

        Server::bind(&self.settings.address)
            .serve(app.into_make_service())
            .await
    }
}
