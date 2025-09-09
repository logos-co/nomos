use std::{
    fmt::{Debug, Display},
    time::Duration,
};

use axum::{
    http::{
        header::{CONTENT_TYPE, USER_AGENT},
        HeaderValue,
    },
    routing::post,
    Router,
};
use kzgrs_backend::common::share::DaShare;
use nomos_api::Backend;
use nomos_da_network_service::backends::libp2p::executor::DaNetworkExecutorBackend;
use nomos_da_sampling::{
    backend::kzgrs::KzgrsSamplingBackend,
    network::adapters::executor::Libp2pAdapter as SamplingLibp2pAdapter,
    storage::adapters::rocksdb::{
        converter::DaStorageConverter, RocksAdapter as SamplingStorageAdapter,
    },
};
use nomos_http_api_common::paths::{DA_GET_MEMBERSHIP, DA_HISTORIC_SAMPLING, UPDATE_MEMBERSHIP};
use nomos_membership::MembershipService as MembershipServiceTrait;
use nomos_node::{
    api::testing::handlers::{da_get_membership, da_historic_sampling, update_membership},
    generic_services::{
        self, DaMembershipAdapter, MembershipBackend, MembershipSdp, MembershipService,
    },
    DaNetworkApiAdapter, NomosDaMembership, Wire,
};
use overwatch::{overwatch::handle::OverwatchHandle, services::AsServiceId, DynError};
use services_utils::wait_until_services_are_ready;
use tokio::net::TcpListener;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use crate::{api::backend::AxumBackendSettings, DaMembershipStorage};

pub struct TestAxumBackend {
    settings: AxumBackendSettings,
}

type TestDaNetworkService<RuntimeServiceId> = nomos_da_network_service::NetworkService<
    DaNetworkExecutorBackend<NomosDaMembership>,
    NomosDaMembership,
    DaMembershipAdapter<RuntimeServiceId>,
    DaMembershipStorage,
    DaNetworkApiAdapter,
    RuntimeServiceId,
>;

type TestDaSamplingService<RuntimeServiceId> = generic_services::DaSamplingService<
    SamplingLibp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        DaNetworkApiAdapter,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

#[async_trait::async_trait]
impl<RuntimeServiceId> Backend<RuntimeServiceId> for TestAxumBackend
where
    RuntimeServiceId: Sync
        + Send
        + Display
        + Debug
        + Clone
        + 'static
        + AsServiceId<MembershipService<RuntimeServiceId>>
        + AsServiceId<TestDaNetworkService<RuntimeServiceId>>
        + AsServiceId<TestDaSamplingService<RuntimeServiceId>>,
{
    type Error = std::io::Error;
    type Settings = AxumBackendSettings;

    async fn new(settings: Self::Settings) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        Ok(Self { settings })
    }

    async fn wait_until_ready(
        &mut self,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    ) -> Result<(), DynError> {
        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            MembershipServiceTrait<_, _, _>
        )
        .await?;
        Ok(())
    }

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

        // Simple router with ONLY testing endpoints
        let app = Router::new()
            .layer(
                builder
                    .allow_headers([CONTENT_TYPE, USER_AGENT])
                    .allow_methods(Any),
            )
            .layer(TraceLayer::new_for_http())
            .route(
                UPDATE_MEMBERSHIP,
                post(
                    update_membership::<
                        MembershipBackend,
                        MembershipSdp<RuntimeServiceId>,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                DA_GET_MEMBERSHIP,
                post(
                    da_get_membership::<
                        DaNetworkExecutorBackend<NomosDaMembership>,
                        NomosDaMembership,
                        DaMembershipAdapter<RuntimeServiceId>,
                        DaMembershipStorage,
                        DaNetworkApiAdapter,
                        RuntimeServiceId,
                    >,
                ),
            )
            .route(
                DA_HISTORIC_SAMPLING,
                post(
                    da_historic_sampling::<
                        KzgrsSamplingBackend,
                        nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
                            NomosDaMembership,
                            DaMembershipAdapter<RuntimeServiceId>,
                            DaMembershipStorage,
                            DaNetworkApiAdapter,
                            RuntimeServiceId,
                        >,
                        SamplingStorageAdapter<DaShare, Wire, DaStorageConverter>,
                        RuntimeServiceId,
                    >,
                ),
            )
            .with_state(handle);

        let listener = TcpListener::bind(&self.settings.address)
            .await
            .expect("Failed to bind address");

        axum::serve(listener, app).await
    }
}
