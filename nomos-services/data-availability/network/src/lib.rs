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
pub struct NetworkConfig<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId, Membership>
where
    Membership: Clone,
{
    pub backend: B::Settings,
    pub membership: Membership,
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId, Membership> Debug
    for NetworkConfig<B, RuntimeServiceId, Membership>
where
    Membership: Clone + Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "NetworkConfig {{ backend: {:?}}}", self.backend)
    }
}

pub struct NetworkService<
    B: NetworkBackend<RuntimeServiceId> + Send + 'static,
    RuntimeServiceId,
    Membership,
> where
    Membership: Clone,
{
    backend: B,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    membership: DaMembershipHandler<Membership>,
}

pub struct NetworkState<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId, Membership> {
    backend: B::State,
    _membership: PhantomData<Membership>,
}

impl<B: NetworkBackend<RuntimeServiceId> + 'static + Send, RuntimeServiceId, Membership> ServiceData
    for NetworkService<B, RuntimeServiceId, Membership>
where
    Membership: Clone,
{
    type Settings = NetworkConfig<B, RuntimeServiceId, Membership>;
    type State = NetworkState<B, RuntimeServiceId, Membership>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaNetworkMsg<B, RuntimeServiceId>;
}

#[async_trait]
impl<B, RuntimeServiceId, Membership> ServiceCore<RuntimeServiceId>
    for NetworkService<B, RuntimeServiceId, Membership>
where
    B: NetworkBackend<RuntimeServiceId> + Send + 'static,
    B::State: Send + Sync,
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
            backend: <B as NetworkBackend<RuntimeServiceId>>::new(
                settings.backend,
                service_resources_handle.overwatch_handle.clone(),
            ),
            service_resources_handle,
            membership,
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
            ref mut membership,
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

impl<Backend, RuntimeServiceId, Membership> Drop
    for NetworkService<Backend, RuntimeServiceId, Membership>
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Membership: Clone,
{
    fn drop(&mut self) {
        self.backend.shutdown();
    }
}

impl<B, RuntimeServiceId, Membership> NetworkService<B, RuntimeServiceId, Membership>
where
    B: NetworkBackend<RuntimeServiceId> + Send + 'static,
    B::State: Send + Sync,
    Membership: Clone + Send + 'static,
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

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId, Membership> Clone
    for NetworkConfig<B, RuntimeServiceId, Membership>
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

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId, Membership> Clone
    for NetworkState<B, RuntimeServiceId, Membership>
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            _membership: PhantomData,
        }
    }
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId, Membership> ServiceState
    for NetworkState<B, RuntimeServiceId, Membership>
where
    Membership: Clone,
{
    type Settings = NetworkConfig<B, RuntimeServiceId, Membership>;
    type Error = <B::State as ServiceState>::Error;

    fn from_settings(settings: &Self::Settings) -> Result<Self, Self::Error> {
        B::State::from_settings(&settings.backend).map(|backend| Self {
            backend,
            _membership: PhantomData,
        })
    }
}
