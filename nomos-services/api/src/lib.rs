use std::{future::Future, time::Duration};

use overwatch::{
    overwatch::handle::OverwatchHandle,
    services::{
        relay::NoMessage,
        state::{NoOperator, NoState},
        ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};

pub mod http;

pub(crate) const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// A simple abstraction so that we can easily
/// change the underlying http server
#[async_trait::async_trait]
pub trait Backend {
    type Error: std::error::Error + Send + Sync + 'static;
    type Settings: Clone + Send + Sync + 'static;

    async fn new(settings: Self::Settings) -> Result<Self, Self::Error>
    where
        Self: Sized;

    async fn serve(self, handle: OverwatchHandle) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiServiceSettings<S> {
    pub backend_settings: S,
}

pub struct ApiService<B: Backend> {
    settings: ApiServiceSettings<B::Settings>,
    handle: OverwatchHandle,
}

impl<B: Backend> ServiceData for ApiService<B> {
    const SERVICE_ID: overwatch::services::ServiceId = "nomos-api";

    type Settings = ApiServiceSettings<B::Settings>;

    type State = NoState<Self::Settings>;

    type StateOperator = NoOperator<Self::State, Self::Settings>;

    type Message = NoMessage;
}

#[async_trait::async_trait]
impl<B> ServiceCore for ApiService<B>
where
    B: Backend + Send + Sync + 'static,
{
    /// Initialize the service with the given state
    fn init(
        service_state: OpaqueServiceStateHandle<Self>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
        let settings = service_state.settings_reader.get_updated_settings();
        Ok(Self {
            settings,
            handle: service_state.overwatch_handle,
        })
    }

    /// Service main loop
    async fn run(mut self) -> Result<(), DynError> {
        let endpoint = B::new(self.settings.backend_settings).await?;
        endpoint.serve(self.handle).await?;
        Ok(())
    }
}

pub(crate) async fn wait_with_timeout<T, F, E>(
    future: F,
    timeout_duration: Duration,
    timeout_error: impl Into<Box<dyn std::error::Error + Send + Sync>>,
) -> Result<T, DynError>
where
    F: Future<Output = Result<T, E>>,
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    tokio::time::timeout(timeout_duration, future)
        .await
        .map_or_else(
            |_| Err(timeout_error.into()),
            |result| result.map_err(std::convert::Into::into),
        )
}
