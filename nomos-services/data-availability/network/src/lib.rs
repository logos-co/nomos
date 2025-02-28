pub mod backends;

use std::{
    fmt::{self, Debug},
    pin::Pin,
};

use async_trait::async_trait;
use backends::NetworkBackend;
use futures::{Stream, StreamExt};
use overwatch_rs::{
    services::{
        relay::RelayMessage,
        state::{NoOperator, ServiceState},
        ServiceCore, ServiceData, ServiceId,
    },
    OpaqueServiceStateHandle,
};
use serde::{Deserialize, Serialize};
use services_utils::overwatch::lifecycle;
use tokio::sync::oneshot;

const DA_NETWORK_TAG: ServiceId = "DA-Network";

pub enum DaNetworkMsg<B: NetworkBackend> {
    Process(B::Message),
    Subscribe {
        kind: B::EventKind,
        sender: oneshot::Sender<Pin<Box<dyn Stream<Item = B::NetworkEvent> + Send>>>,
    },
}

impl<B: NetworkBackend> Debug for DaNetworkMsg<B> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Process(msg) => write!(fmt, "DaNetworkMsg::Process({msg:?})"),
            Self::Subscribe { kind, .. } => {
                write!(fmt, "DaNetworkMsg::Subscribe{{ kind: {kind:?}}}")
            }
        }
    }
}

impl<T: NetworkBackend + 'static> RelayMessage for DaNetworkMsg<T> {}

#[derive(Serialize, Deserialize)]
pub struct NetworkConfig<B: NetworkBackend> {
    pub backend: B::Settings,
}

impl<B: NetworkBackend> Debug for NetworkConfig<B> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "NetworkConfig {{ backend: {:?}}}", self.backend)
    }
}

pub struct NetworkService<B: NetworkBackend + Send + 'static> {
    backend: B,
    service_state: OpaqueServiceStateHandle<Self>,
}

pub struct NetworkState<B: NetworkBackend> {
    _backend: B::State,
}

impl<B: NetworkBackend + 'static + Send> ServiceData for NetworkService<B> {
    const SERVICE_ID: ServiceId = DA_NETWORK_TAG;
    type Settings = NetworkConfig<B>;
    type State = NetworkState<B>;
    type StateOperator = NoOperator<Self::State, Self::Settings>;
    type Message = DaNetworkMsg<B>;
}

#[async_trait]
impl<B> ServiceCore for NetworkService<B>
where
    B: NetworkBackend + Send + 'static,
    B::State: Send + Sync,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self>,
        _init_state: Self::State,
    ) -> Result<Self, overwatch_rs::DynError> {
        Ok(Self {
            backend: <B as NetworkBackend>::new(
                service_state.settings_reader.get_updated_settings().backend,
                service_state.overwatch_handle.clone(),
            ),
            service_state,
        })
    }

    async fn run(mut self) -> Result<(), overwatch_rs::DynError> {
        let Self {
            service_state:
                OpaqueServiceStateHandle::<Self> {
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
                    if lifecycle::should_stop_service::<Self>(&msg).await {
                        // TODO: Maybe add a call to backend to handle this. Maybe trying to save unprocessed messages?
                        backend.shutdown();
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

impl<B> NetworkService<B>
where
    B: NetworkBackend + Send + 'static,
    B::State: Send + Sync,
{
    async fn handle_network_service_message(msg: DaNetworkMsg<B>, backend: &mut B) {
        match msg {
            DaNetworkMsg::Process(msg) => {
                // split sending in two steps to help the compiler understand we do not
                // need to hold an instance of &I (which is not send) across an await point
                let _send = backend.process(msg);
                _send.await
            }
            DaNetworkMsg::Subscribe { kind, sender } => sender
                .send(backend.subscribe(kind).await)
                .unwrap_or_else(|_| {
                    tracing::warn!(
                        "client hung up before a subscription handle could be established"
                    )
                }),
        }
    }
}

impl<B: NetworkBackend> Clone for NetworkConfig<B> {
    fn clone(&self) -> Self {
        NetworkConfig {
            backend: self.backend.clone(),
        }
    }
}

impl<B: NetworkBackend> Clone for NetworkState<B> {
    fn clone(&self) -> Self {
        NetworkState {
            _backend: self._backend.clone(),
        }
    }
}

impl<B: NetworkBackend> ServiceState for NetworkState<B> {
    type Settings = NetworkConfig<B>;
    type Error = <B::State as ServiceState>::Error;

    fn from_settings(settings: &Self::Settings) -> Result<Self, Self::Error> {
        B::State::from_settings(&settings.backend).map(|_backend| Self { _backend })
    }
}
