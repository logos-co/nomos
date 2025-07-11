use std::{fmt::Display, future::Future, sync::OnceLock, time::Duration};

use overwatch::{
    overwatch::handle::OverwatchHandle,
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
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

    /// Wait until the backend is ready to serve requests.
    ///
    /// # Notes
    ///
    /// * Because this method is async and takes a `self` reference, it would
    ///   need to propagate `Sync` traits. To avoid this, we use a `&mut self`
    ///   reference. In addition, this also covers potential use-cases where the
    ///   backend might need to perform some `mut` operations when checking
    ///   readiness, such as sending/receiving messages.
    ///
    /// # Arguments
    ///
    /// * `overwatch_handle` - A handle to Overwatch. This is mainly used to
    ///   retrieve the
    ///   [`StatusWatcher`](overwatch::services::status::StatusWatcher) to read
    ///   the [`ServiceStatus`](overwatch::services::status::ServiceStatus) of
    ///   `Services` we depend on.
    ///
    /// # Returns
    ///
    /// Returns a `Result` indicating:
    /// * `Ok(())` if the backend is ready.
    /// * `Err(DynError)` if there was an error while waiting for readiness.
    async fn wait_until_ready(
        &mut self,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    ) -> Result<(), DynError>;

    async fn serve(self, handle: OverwatchHandle<RuntimeServiceId>) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiServiceSettings<S> {
    pub backend_settings: S,
    #[serde(default)]
    pub request_timeout: Option<Duration>,
}

pub struct ApiService<B: Backend<RuntimeServiceId>, RuntimeServiceId> {
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    settings: ApiServiceSettings<B::Settings>,
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
    RuntimeServiceId: AsServiceId<Self> + Display + Send + Clone,
{
    /// Initialize the service with the given state
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        if let Some(timeout) = &settings.request_timeout {
            let _ = HTTP_REQUEST_TIMEOUT.set(*timeout);
        }

        Ok(Self {
            service_resources_handle,
            settings,
        })
    }

    /// Service main loop
    async fn run(mut self) -> Result<(), DynError> {
        let mut endpoint = B::new(self.settings.backend_settings).await?;

        self.service_resources_handle.status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        endpoint
            .wait_until_ready(self.service_resources_handle.overwatch_handle.clone())
            .await?;

        endpoint
            .serve(self.service_resources_handle.overwatch_handle)
            .await?;

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
            |result| result.map_err(Into::into),
        )
}
