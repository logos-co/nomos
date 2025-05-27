use std::fmt::{Debug, Display};

use async_trait::async_trait;
use backends::NetworkBackend;
use futures::StreamExt as _;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceStateHandle,
};
use services_utils::overwatch::lifecycle;

use crate::{config::NetworkConfig, message::BackendNetworkMsg};

pub mod backends;
pub mod config;
pub mod message;

pub struct NetworkService<Backend, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId> + 'static,
{
    backend: Backend,
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
}

impl<Backend, RuntimeServiceId> ServiceData for NetworkService<Backend, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId> + 'static,
{
    type Settings = NetworkConfig<Backend::Settings>;
    type State = NoState<NetworkConfig<Backend::Settings>>;
    type StateOperator = NoOperator<Self::State>;
    type Message = BackendNetworkMsg<Backend, RuntimeServiceId>;
}

#[async_trait]
impl<Backend, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for NetworkService<Backend, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            backend: Backend::new(
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

impl<Backend, RuntimeServiceId> NetworkService<Backend, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
{
    async fn handle_network_service_message(
        msg: BackendNetworkMsg<Backend, RuntimeServiceId>,
        backend: &mut Backend,
    ) {
        match msg {
            BackendNetworkMsg::<Backend, _>::Process(msg) => {
                // split sending in two steps to help the compiler understand we do not
                // need to hold an instance of &I (which is not send) across an await point
                let send = backend.process(msg);
                send.await;
            }
            BackendNetworkMsg::<Backend, _>::Subscribe { kind, sender } => sender
                .send(backend.subscribe(kind).await)
                .unwrap_or_else(|_| {
                    tracing::warn!(
                        "client hung up before a subscription handle could be established"
                    );
                }),
        }
    }
}
