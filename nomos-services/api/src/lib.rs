use std::{future::Future, sync::OnceLock, time::Duration};

use overwatch::{
    overwatch::handle::OverwatchHandle,
    services::{
        state::{NoOperator, NoState},
        ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
pub mod http;

static HTTP_REQUEST_TIMEOUT: OnceLock<Duration> = OnceLock::new();

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// A simple abstraction so that we can easily
/// change the underlying http server
#[async_trait::async_trait]
pub trait Backend<RuntimeServiceId> {
    type Error: std::error::Error + Send + Sync + 'static;
    type Settings: Clone + Send + Sync + 'static;

    async fn new(settings: Self::Settings) -> Result<Self, Self::Error>
    where
        Self: Sized;

    async fn serve(self, handle: OverwatchHandle<RuntimeServiceId>) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiServiceSettings<S> {
    pub backend_settings: S,
    #[serde(default)]
    pub request_timeout: Option<Duration>,
}

pub struct ApiService<B: Backend<RuntimeServiceId>, RuntimeServiceId> {
    settings: ApiServiceSettings<B::Settings>,
    handle: OverwatchHandle<RuntimeServiceId>,
}

impl<B: Backend<RuntimeServiceId>, RuntimeServiceId> ServiceData
    for ApiService<B, RuntimeServiceId>
{
    type Settings = ApiServiceSettings<B::Settings>;

    type State = NoState<Self::Settings>;

    type StateOperator = NoOperator<Self::State>;

    type Message = ();
}

#[async_trait::async_trait]
impl<B, RuntimeServiceId> ServiceCore<RuntimeServiceId> for ApiService<B, RuntimeServiceId>
where
    B: Backend<RuntimeServiceId> + Send + Sync + 'static,
    RuntimeServiceId: Send,
{
    /// Initialize the service with the given state
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
        let settings = service_state.settings_reader.get_updated_settings();

        if let Some(timeout) = &settings.request_timeout {
            let _ = HTTP_REQUEST_TIMEOUT.set(*timeout);
        }

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
    timeout_error: impl Into<Box<dyn std::error::Error + Send + Sync>>,
) -> Result<T, DynError>
where
    F: Future<Output = Result<T, E>>,
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let request_timeout = HTTP_REQUEST_TIMEOUT
        .get()
        .unwrap_or(&DEFAULT_REQUEST_TIMEOUT);

    tokio::time::timeout(*request_timeout, future)
        .await
        .map_or_else(
            |_| Err(timeout_error.into()),
            |result| result.map_err(std::convert::Into::into),
        )
}
