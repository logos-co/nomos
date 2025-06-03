pub mod backends;

use std::{
    fmt::{self, Debug, Display},
    pin::Pin,
};

use async_trait::async_trait;
use backends::NetworkBackend;
use futures::Stream;
use overwatch::{
    services::{
        state::{NoOperator, ServiceState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

pub enum DaNetworkMsg<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> {
    Process(B::Message),
    Subscribe {
        kind: B::EventKind,
        sender: oneshot::Sender<Pin<Box<dyn Stream<Item = B::NetworkEvent> + Send>>>,
    },
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> Debug
    for DaNetworkMsg<B, RuntimeServiceId>
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Process(msg) => write!(fmt, "DaNetworkMsg::Process({msg:?})"),
            Self::Subscribe { kind, .. } => {
                write!(fmt, "DaNetworkMsg::Subscribe{{ kind: {kind:?}}}")
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct NetworkConfig<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> {
    pub backend: B::Settings,
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> Debug
    for NetworkConfig<B, RuntimeServiceId>
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "NetworkConfig {{ backend: {:?}}}", self.backend)
    }
}

pub struct NetworkService<B: NetworkBackend<RuntimeServiceId> + Send + 'static, RuntimeServiceId> {
    backend: B,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
}

pub struct NetworkState<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> {
    backend: B::State,
}

impl<B: NetworkBackend<RuntimeServiceId> + 'static + Send, RuntimeServiceId> ServiceData
    for NetworkService<B, RuntimeServiceId>
{
    type Settings = NetworkConfig<B, RuntimeServiceId>;
    type State = NetworkState<B, RuntimeServiceId>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaNetworkMsg<B, RuntimeServiceId>;
}

#[async_trait]
impl<B, RuntimeServiceId> ServiceCore<RuntimeServiceId> for NetworkService<B, RuntimeServiceId>
where
    B: NetworkBackend<RuntimeServiceId> + Send + 'static,
    B::State: Send + Sync,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            backend: <B as NetworkBackend<RuntimeServiceId>>::new(
                service_resources_handle
                    .settings_handle
                    .notifier()
                    .get_updated_settings()
                    .backend,
                service_resources_handle.overwatch_handle.clone(),
            ),
            service_resources_handle,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            service_resources_handle:
                OpaqueServiceResourcesHandle::<Self, RuntimeServiceId> {
                    ref mut inbound_relay,
                    ref status_updater,
                    ..
                },
            ref mut backend,
        } = self;

        status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        while let Some(msg) = inbound_relay.recv().await {
            Self::handle_network_service_message(msg, backend).await;
        }

        Ok(())
    }
}

impl<Backend, RuntimeServiceId> Drop for NetworkService<Backend, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
{
    fn drop(&mut self) {
        self.backend.shutdown();
    }
}

impl<B, RuntimeServiceId> NetworkService<B, RuntimeServiceId>
where
    B: NetworkBackend<RuntimeServiceId> + Send + 'static,
    B::State: Send + Sync,
{
    async fn handle_network_service_message(
        msg: DaNetworkMsg<B, RuntimeServiceId>,
        backend: &mut B,
    ) {
        match msg {
            DaNetworkMsg::Process(msg) => {
                // split sending in two steps to help the compiler understand we do not
                // need to hold an instance of &I (which is not Send) across an await point
                let send = backend.process(msg);
                send.await;
            }
            DaNetworkMsg::Subscribe { kind, sender } => sender
                .send(backend.subscribe(kind).await)
                .unwrap_or_else(|_| {
                    tracing::warn!(
                        "client hung up before a subscription handle could be established"
                    );
                }),
        }
    }
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> Clone
    for NetworkConfig<B, RuntimeServiceId>
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
        }
    }
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> Clone
    for NetworkState<B, RuntimeServiceId>
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
        }
    }
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> ServiceState
    for NetworkState<B, RuntimeServiceId>
{
    type Settings = NetworkConfig<B, RuntimeServiceId>;
    type Error = <B::State as ServiceState>::Error;

    fn from_settings(settings: &Self::Settings) -> Result<Self, Self::Error> {
        B::State::from_settings(&settings.backend).map(|backend| Self { backend })
    }
}
