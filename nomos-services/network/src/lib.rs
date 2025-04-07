pub mod backends;
use std::fmt::{self, Debug, Display};

use async_trait::async_trait;
use backends::NetworkBackend;
use futures::StreamExt;
use overwatch::{
    services::{
        state::{NoOperator, ServiceState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceStateHandle,
};
use serde::{Deserialize, Serialize};
use services_utils::overwatch::lifecycle;
use tokio::sync::{broadcast, oneshot};

pub enum NetworkMsg<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> {
    Process(B::Message),
    Subscribe {
        kind: B::EventKind,
        sender: oneshot::Sender<broadcast::Receiver<B::NetworkEvent>>,
    },
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> Debug
    for NetworkMsg<B, RuntimeServiceId>
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Process(msg) => write!(fmt, "NetworkMsg::Process({msg:?})"),
            Self::Subscribe { kind, sender } => write!(
                fmt,
                "NetworkMsg::Subscribe{{ kind: {kind:?}, sender: {sender:?}}}"
            ),
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

pub struct NetworkService<B: NetworkBackend<RuntimeServiceId> + 'static, RuntimeServiceId> {
    backend: B,
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
}

pub struct NetworkState<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> {
    backend: B::State,
}

impl<B: NetworkBackend<RuntimeServiceId> + 'static, RuntimeServiceId> ServiceData
    for NetworkService<B, RuntimeServiceId>
{
    type Settings = NetworkConfig<B, RuntimeServiceId>;
    type State = NetworkState<B, RuntimeServiceId>;
    type StateOperator = NoOperator<Self::State>;
    type Message = NetworkMsg<B, RuntimeServiceId>;
}

#[async_trait]
impl<B, RuntimeServiceId> ServiceCore<RuntimeServiceId> for NetworkService<B, RuntimeServiceId>
where
    B: NetworkBackend<RuntimeServiceId> + Send + 'static,
    B::State: Send + Sync,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            backend: <B as NetworkBackend<RuntimeServiceId>>::new(
                service_state.settings_reader.get_updated_settings().backend,
                service_state.overwatch_handle.clone(),
            ),
            service_state,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            service_state:
                OpaqueServiceStateHandle::<Self, RuntimeServiceId> {
                    mut inbound_relay,
                    lifecycle_handle,
                    ..
                },
            mut backend,
        } = self;
        let mut lifecycle_stream = lifecycle_handle.message_stream();
        loop {
            tokio::select! {
                Some(msg) = inbound_relay.recv() => {
                    Self::handle_network_service_message(msg, &mut backend).await;
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
                        // TODO: Maybe add a call to backend to handle this. Maybe trying to save unprocessed messages?
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

impl<B, RuntimeServiceId> NetworkService<B, RuntimeServiceId>
where
    B: NetworkBackend<RuntimeServiceId> + Send + 'static,
    B::State: Send + Sync,
{
    async fn handle_network_service_message(msg: NetworkMsg<B, RuntimeServiceId>, backend: &mut B) {
        match msg {
            NetworkMsg::Process(msg) => {
                // split sending in two steps to help the compiler understand we do not
                // need to hold an instance of &I (which is not send) across an await point
                let send = backend.process(msg);
                send.await;
            }
            NetworkMsg::Subscribe { kind, sender } => sender
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
