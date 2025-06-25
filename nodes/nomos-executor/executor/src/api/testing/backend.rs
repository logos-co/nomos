use std::{
    fmt::{Debug, Display},
    time::Duration,
};

use axum::{http::HeaderValue, routing::post, Router, Server};
use hyper::header::{CONTENT_TYPE, USER_AGENT};
use nomos_api::Backend;
use nomos_http_api_common::paths::UPDATE_MEMBERSHIP;
use nomos_membership::MembershipService as MembershipServiceTrait;
use nomos_node::{
    api::testing::handlers::update_membership,
    generic_services::{MembershipBackend, MembershipSdp, MembershipService},
};
use overwatch::{overwatch::handle::OverwatchHandle, services::AsServiceId, DynError};
use services_utils::wait_until_services_are_ready;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use crate::api::backend::AxumBackendSettings;

pub struct TestAxumBackend {
    settings: AxumBackendSettings,
}

#[async_trait::async_trait]
impl<RuntimeServiceId> Backend<RuntimeServiceId> for TestAxumBackend
where
    RuntimeServiceId: Sync
        + Send
        + Display
        + Debug
        + Clone
        + 'static
        + AsServiceId<MembershipService<RuntimeServiceId>>,
{
    type Error = hyper::Error;
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
            .with_state(handle);

        Server::bind(&self.settings.address)
            .serve(app.into_make_service())
            .await
    }
}
