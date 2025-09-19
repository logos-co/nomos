#![allow(clippy::needless_for_each, reason = "Utoipa implementation")]

use std::{
    error::Error,
    fmt::{Debug, Display},
    hash::Hash,
    time::Duration,
};

use axum::{
    http::{
        header::{CONTENT_TYPE, USER_AGENT},
        HeaderValue,
    },
    routing, Router,
};
use broadcast_service::BlockBroadcastService;
use nomos_api::{
    http::{consensus::Cryptarchia, da::DaVerifier, storage},
    Backend,
};
use nomos_blend_service::{ProofsGenerator, ProofsVerifier};
use nomos_core::{
    da::{
        blob::{LightShare, Share},
        BlobId, DaVerifier as CoreDaVerifier,
    },
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, SignedMantleTx, Transaction},
};
use nomos_da_network_core::SubnetworkId;
use nomos_da_network_service::{
    backends::libp2p::validator::DaNetworkValidatorBackend, membership::MembershipAdapter,
    storage::MembershipStorageAdapter,
};
use nomos_da_sampling::{backend::DaSamplingServiceBackend, DaSamplingService};
use nomos_da_verifier::{backend::VerifierBackend, mempool::DaMempoolAdapter};
pub use nomos_http_api_common::settings::AxumBackendSettings;
use nomos_http_api_common::{paths, utils::create_rate_limit_layer};
use nomos_libp2p::PeerId;
use nomos_mempool::{
    backend::mockpool::MockPool, tx::service::openapi::Status, MempoolMetrics, TxMempoolService,
};
use nomos_storage::{api::da::DaConverter, backends::rocksdb::RocksBackend, StorageService};
use overwatch::{overwatch::handle::OverwatchHandle, services::AsServiceId, DynError};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use subnetworks_assignations::MembershipHandler;
use tokio::net::TcpListener;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use super::handlers::{
    add_share, add_tx, balancer_stats, blacklisted_peers, block, block_peer, cl_metrics, cl_status,
    cryptarchia_headers, cryptarchia_info, cryptarchia_lib_stream, da_get_commitments,
    da_get_light_share, da_get_shares, da_get_storage_commitments, libp2p_info, monitor_stats,
    unblock_peer,
};

pub(crate) type DaStorageBackend = RocksBackend;
type DaStorageService<RuntimeServiceId> = StorageService<DaStorageBackend, RuntimeServiceId>;

pub struct AxumBackend<
    DaShare,
    Membership,
    DaMembershipAdapter,
    DaMembershipStorage,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
    Tx,
    DaStorageConverter,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    VerifierMempoolAdapter,
    TimeBackend,
    ApiAdapter,
    HttpStorageAdapter,
    BlendProofsGenerator,
    BlendProofsVerifier,
    const SIZE: usize,
> {
    settings: AxumBackendSettings,
    _share: core::marker::PhantomData<DaShare>,
    _membership: core::marker::PhantomData<Membership>,
    _verifier_backend: core::marker::PhantomData<DaVerifierBackend>,
    _verifier_network: core::marker::PhantomData<DaVerifierNetwork>,
    _verifier_storage: core::marker::PhantomData<DaVerifierStorage>,
    _tx: core::marker::PhantomData<Tx>,
    _storage_converter: core::marker::PhantomData<DaStorageConverter>,
    _sampling_backend: core::marker::PhantomData<SamplingBackend>,
    _sampling_network_adapter: core::marker::PhantomData<SamplingNetworkAdapter>,
    _sampling_storage: core::marker::PhantomData<SamplingStorage>,
    _time_backend: core::marker::PhantomData<TimeBackend>,
    _api_adapter: core::marker::PhantomData<ApiAdapter>,
    _storage_adapter: core::marker::PhantomData<HttpStorageAdapter>,
    _da_membership: core::marker::PhantomData<(DaMembershipAdapter, DaMembershipStorage)>,
    _verifier_mempool_adapter: core::marker::PhantomData<VerifierMempoolAdapter>,
    _blend: core::marker::PhantomData<(BlendProofsGenerator, BlendProofsVerifier)>,
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
        DaShare,
        Membership,
        DaMembershipAdapter,
        DaMembershipStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        Tx,
        DaStorageConverter,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        VerifierMempoolAdapter,
        TimeBackend,
        ApiAdapter,
        StorageAdapter,
        BlendProofsGenerator,
        BlendProofsVerifier,
        const SIZE: usize,
        RuntimeServiceId,
    > Backend<RuntimeServiceId>
    for AxumBackend<
        DaShare,
        Membership,
        DaMembershipAdapter,
        DaMembershipStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        Tx,
        DaStorageConverter,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        VerifierMempoolAdapter,
        TimeBackend,
        ApiAdapter,
        StorageAdapter,
        BlendProofsGenerator,
        BlendProofsVerifier,
        SIZE,
    >
where
    DaShare: Share + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    <DaShare as Share>::BlobId: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    <DaShare as Share>::ShareIndex: Serialize + DeserializeOwned + Send + Sync + 'static,
    DaShare::LightShare: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    DaShare::SharesCommitments: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    DaMembershipAdapter: MembershipAdapter + Send + Sync + 'static,
    DaMembershipStorage: MembershipStorageAdapter<PeerId, SubnetworkId> + Send + Sync + 'static,
    DaVerifierBackend: VerifierBackend + CoreDaVerifier<DaShare = DaShare> + Send + Sync + 'static,
    <DaVerifierBackend as VerifierBackend>::Settings: Clone,
    <DaVerifierBackend as CoreDaVerifier>::Error: Error,
    DaVerifierNetwork:
        nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
    DaVerifierStorage:
        nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync + 'static,
    Tx: AuthenticatedMantleTx
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
    SamplingBackend: DaSamplingServiceBackend<BlobId = BlobId> + Send + 'static,
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
    <DaShare as Share>::ShareIndex: Clone + Hash + Eq + Send + Sync + 'static,
    DaShare::LightShare: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    DaShare::SharesCommitments: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
    SamplingStorage:
        nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync + 'static,
    DaVerifierNetwork::Settings: Clone,
    VerifierMempoolAdapter: DaMempoolAdapter + Send + Sync + 'static,
    TimeBackend: nomos_time::backends::TimeBackend + Send + 'static,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_network_service::api::ApiAdapter + Send + Sync + 'static,
    DaStorageConverter:
        DaConverter<DaStorageBackend, Share = DaShare, Tx = SignedMantleTx> + Send + Sync + 'static,
    StorageAdapter: storage::StorageAdapter<RuntimeServiceId> + Send + Sync + 'static,
    BlendProofsGenerator: ProofsGenerator + Send + 'static,
    BlendProofsVerifier: ProofsVerifier + Clone + Send + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Send
        + Display
        + Clone
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
        >
        + AsServiceId<BlockBroadcastService<RuntimeServiceId>>
        + AsServiceId<
            DaVerifier<
                DaShare,
                DaVerifierNetwork,
                DaVerifierBackend,
                DaStorageConverter,
                VerifierMempoolAdapter,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<
            nomos_da_network_service::NetworkService<
                DaNetworkValidatorBackend<Membership>,
                Membership,
                DaMembershipAdapter,
                DaMembershipStorage,
                ApiAdapter,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<
            nomos_network::NetworkService<
                nomos_network::backends::libp2p::Libp2p,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<DaStorageService<RuntimeServiceId>>
        + AsServiceId<
            TxMempoolService<
                nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
                    Tx,
                    <Tx as Transaction>::Hash,
                    RuntimeServiceId,
                >,
                SamplingNetworkAdapter,
                SamplingStorage,
                MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<
            DaSamplingService<
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                RuntimeServiceId,
            >,
        >,
{
    type Error = std::io::Error;
    type Settings = AxumBackendSettings;

    async fn new(settings: Self::Settings) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        Ok(Self {
            settings,
            _share: core::marker::PhantomData,
            _membership: core::marker::PhantomData,
            _verifier_backend: core::marker::PhantomData,
            _verifier_network: core::marker::PhantomData,
            _verifier_storage: core::marker::PhantomData,
            _tx: core::marker::PhantomData,
            _storage_converter: core::marker::PhantomData,
            _sampling_backend: core::marker::PhantomData,
            _sampling_network_adapter: core::marker::PhantomData,
            _sampling_storage: core::marker::PhantomData,
            _time_backend: core::marker::PhantomData,
            _api_adapter: core::marker::PhantomData,
            _storage_adapter: core::marker::PhantomData,
            _da_membership: core::marker::PhantomData,
            _verifier_mempool_adapter: core::marker::PhantomData,
            _blend: core::marker::PhantomData,
        })
    }

    async fn wait_until_ready(
        &mut self,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    ) -> Result<(), DynError> {
        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            Cryptarchia<
                _,
                _,
                _,
                _,
                _,
                _,
                _,
                _,
                SIZE,
            >,
            DaVerifier<_, _, _, _, _, _>,
            nomos_da_network_service::NetworkService<_, _, _, _, _, _>,
            nomos_network::NetworkService<_, _>,
            DaStorageService<_>,
            TxMempoolService<_, _, _, _, _>
        )
        .await?;
        Ok(())
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
            .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
            .route(
                paths::CL_METRICS,
                routing::get(
                    cl_metrics::<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>,
                ),
            )
            .route(
                paths::CL_STATUS,
                routing::post(
                    cl_status::<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>,
                ),
            )
            .route(
                paths::CRYPTARCHIA_INFO,
                routing::get(
                    cryptarchia_info::<
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
                ),
            )
            .route(
                paths::CRYPTARCHIA_HEADERS,
                routing::get(
                    cryptarchia_headers::<
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
                ),
            )
            .route(
                paths::CRYPTARCHIA_LIB_STREAM,
                routing::get(cryptarchia_lib_stream::<RuntimeServiceId>),
            )
            .route(
                paths::DA_ADD_SHARE,
                routing::post(
                    add_share::<
                        DaShare,
                        DaVerifierNetwork,
                        DaVerifierBackend,
                        DaStorageConverter,
                        VerifierMempoolAdapter,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_BLOCK_PEER,
                routing::post(
                    block_peer::<
                        DaNetworkValidatorBackend<Membership>,
                        Membership,
                        DaMembershipAdapter,
                        DaMembershipStorage,
                        ApiAdapter,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_UNBLOCK_PEER,
                routing::post(
                    unblock_peer::<
                        DaNetworkValidatorBackend<Membership>,
                        Membership,
                        DaMembershipAdapter,
                        DaMembershipStorage,
                        ApiAdapter,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_BLACKLISTED_PEERS,
                routing::get(
                    blacklisted_peers::<
                        DaNetworkValidatorBackend<Membership>,
                        Membership,
                        DaMembershipAdapter,
                        DaMembershipStorage,
                        ApiAdapter,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::NETWORK_INFO,
                routing::get(libp2p_info::<RuntimeServiceId>),
            )
            .route(
                paths::STORAGE_BLOCK,
                routing::post(block::<StorageAdapter, Tx, RuntimeServiceId>),
            )
            .route(
                paths::MEMPOOL_ADD_TX,
                routing::post(
                    add_tx::<Tx, SamplingNetworkAdapter, SamplingStorage, RuntimeServiceId>,
                ),
            )
            .route(
                paths::DA_GET_SHARES_COMMITMENTS,
                routing::post(
                    da_get_commitments::<
                        BlobId,
                        SamplingBackend,
                        SamplingNetworkAdapter,
                        SamplingStorage,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_GET_STORAGE_SHARES_COMMITMENTS,
                routing::get(
                    da_get_storage_commitments::<
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
                    da_get_shares::<DaStorageConverter, StorageAdapter, DaShare, RuntimeServiceId>,
                ),
            )
            .route(
                paths::DA_BALANCER_STATS,
                routing::get(
                    balancer_stats::<
                        DaNetworkValidatorBackend<Membership>,
                        Membership,
                        DaMembershipAdapter,
                        DaMembershipStorage,
                        ApiAdapter,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                paths::DA_MONITOR_STATS,
                routing::get(
                    monitor_stats::<
                        DaNetworkValidatorBackend<Membership>,
                        Membership,
                        DaMembershipAdapter,
                        DaMembershipStorage,
                        ApiAdapter,
                        RuntimeServiceId,
                    >,
                ),
            )
            .with_state(handle)
            .layer(TimeoutLayer::new(self.settings.timeout))
            .layer(RequestBodyLimitLayer::new(self.settings.max_body_size))
            .layer(ConcurrencyLimitLayer::new(
                self.settings.max_concurrent_requests,
            ))
            .layer(create_rate_limit_layer(&self.settings))
            .layer(TraceLayer::new_for_http())
            .layer(
                builder
                    .allow_headers(vec![CONTENT_TYPE, USER_AGENT])
                    .allow_methods(Any),
            );

        let listener = TcpListener::bind(&self.settings.address)
            .await
            .map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("Failed to bind to address {}: {}", self.settings.address, e),
                )
            })?;

        let app = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
        axum::serve(listener, app).await
    }
}
