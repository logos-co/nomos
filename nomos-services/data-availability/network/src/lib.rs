pub mod backends;
pub mod membership;

use std::{
    fmt::{self, Debug, Display},
    marker::PhantomData,
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

use crate::membership::handler::DaMembershipHandler;

pub enum DaNetworkMsg<Backend: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> {
    Process(Backend::Message),
    Subscribe {
        kind: Backend::EventKind,
        sender: oneshot::Sender<Pin<Box<dyn Stream<Item = Backend::NetworkEvent> + Send>>>,
    },
}

impl<Backend: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> Debug
    for DaNetworkMsg<Backend, RuntimeServiceId>
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
pub struct NetworkConfig<Backend: NetworkBackend<RuntimeServiceId>, Membership, RuntimeServiceId> {
    pub backend: Backend::Settings,
    pub membership: Membership,
}

impl<Backend: NetworkBackend<RuntimeServiceId>, Membership, RuntimeServiceId> Debug
    for NetworkConfig<Backend, Membership, RuntimeServiceId>
where
    Membership: Clone + Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "NetworkConfig {{ backend: {:?}}}", self.backend)
    }
}

pub struct NetworkService<
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Membership,
    RuntimeServiceId,
> {
    backend: Backend,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _membership: DaMembershipHandler<Membership>,
}

pub struct NetworkState<Backend: NetworkBackend<RuntimeServiceId>, Membership, RuntimeServiceId> {
    backend: Backend::State,
    _membership: PhantomData<Membership>,
}

impl<Backend: NetworkBackend<RuntimeServiceId> + 'static + Send, Membership, RuntimeServiceId>
    ServiceData for NetworkService<Backend, Membership, RuntimeServiceId>
{
    type Settings = NetworkConfig<Backend, Membership, RuntimeServiceId>;
    type State = NetworkState<Backend, Membership, RuntimeServiceId>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaNetworkMsg<Backend, RuntimeServiceId>;
}

#[async_trait]
impl<Backend, Membership, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for NetworkService<Backend, Membership, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId, Membership = DaMembershipHandler<Membership>>
        + Send
        + 'static,
    Backend::State: Send + Sync,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send,
    Membership: Clone + Send + Sync + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let membership = DaMembershipHandler::new(settings.membership);

        Ok(Self {
            backend: <Backend as NetworkBackend<RuntimeServiceId>>::new(
                settings.backend,
                service_resources_handle.overwatch_handle.clone(),
                membership.clone(),
            ),
            service_resources_handle,
            _membership: membership,
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
            // todo: get membership here for updates
            ..
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

impl<Backend, Membership, RuntimeServiceId> Drop
    for NetworkService<Backend, Membership, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
{
    fn drop(&mut self) {
        self.backend.shutdown();
    }
}

impl<Backend, Membership, RuntimeServiceId> NetworkService<Backend, Membership, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Backend::State: Send + Sync,
    Membership: Clone + Send + 'static,
{
    async fn handle_network_service_message(
        msg: DaNetworkMsg<Backend, RuntimeServiceId>,
        backend: &mut Backend,
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

impl<Backend: NetworkBackend<RuntimeServiceId>, Membership, RuntimeServiceId> Clone
    for NetworkConfig<Backend, Membership, RuntimeServiceId>
where
    Membership: Clone,
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            membership: self.membership.clone(),
        }
    }
}

impl<Backend: NetworkBackend<RuntimeServiceId>, Membership, RuntimeServiceId> Clone
    for NetworkState<Backend, Membership, RuntimeServiceId>
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            _membership: PhantomData,
        }
    }
}

impl<Backend: NetworkBackend<RuntimeServiceId>, Membership, RuntimeServiceId> ServiceState
    for NetworkState<Backend, Membership, RuntimeServiceId>
where
    Membership: Clone,
{
    type Settings = NetworkConfig<Backend, Membership, RuntimeServiceId>;
    type Error = <Backend::State as ServiceState>::Error;

    fn from_settings(settings: &Self::Settings) -> Result<Self, Self::Error> {
        Backend::State::from_settings(&settings.backend).map(|backend| Self {
            backend,
            _membership: PhantomData,
        })
    }
}
