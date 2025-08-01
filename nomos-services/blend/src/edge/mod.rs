mod backends;
mod settings;

use std::{fmt::Display, marker::PhantomData};

use backends::BlendBackend;
use nomos_blend_scheduling::{
    message_blend::crypto::CryptographicProcessor, serialize_encapsulated_message,
};
use nomos_core::wire;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use rand::{RngCore, SeedableRng as _};
use rand_chacha::ChaCha12Rng;
use serde::Serialize;
use settings::BlendConfig;
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt as _};

use crate::message::ServiceMessage;

const LOG_TARGET: &str = "blend::service::edge";

pub struct BlendService<Backend, NodeId, BroadcastSettings, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
{
    backend: Backend,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<BroadcastSettings>,
}

impl<Backend, NodeId, BroadcastSettings, RuntimeServiceId> ServiceData
    for BlendService<Backend, NodeId, BroadcastSettings, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
{
    type Settings = BlendConfig<Backend::Settings, NodeId>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ServiceMessage<BroadcastSettings>;
}

#[async_trait::async_trait]
impl<Backend, NodeId, BroadcastSettings, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<Backend, NodeId, BroadcastSettings, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Send + Sync,
    NodeId: Clone + Send + Sync + 'static,
    BroadcastSettings: Serialize + Send,
    RuntimeServiceId: AsServiceId<Self> + Display + Clone + Send,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        let membership = settings.membership();
        let current_membership = Some(membership.clone());
        let backend = <Backend as BlendBackend<NodeId, RuntimeServiceId>>::new(
            settings.backend,
            service_resources_handle.overwatch_handle.clone(),
            Box::pin(
                IntervalStream::new(interval(settings.time.session_duration()))
                    .map(move |_| membership.clone()),
            ),
            current_membership,
            ChaCha12Rng::from_entropy(),
        );

        Ok(Self {
            backend,
            service_resources_handle,
            _phantom: PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            service_resources_handle:
                OpaqueServiceResourcesHandle::<Self, RuntimeServiceId> {
                    ref mut inbound_relay,
                    ref settings_handle,
                    ref status_updater,
                    ..
                },
            ref mut backend,
            ..
        } = self;

        let settings = settings_handle.notifier().get_updated_settings();
        let mut cryptoraphic_processor = CryptographicProcessor::new(
            settings.crypto.clone(),
            settings.membership(),
            ChaCha12Rng::from_entropy(),
        );

        let mut messages_to_blend = inbound_relay.map(|ServiceMessage::Blend(message)| {
            wire::serialize(&message)
                .expect("Message from internal services should not fail to serialize")
        });

        status_updater.notify_ready();
        tracing::info!(
            target: LOG_TARGET,
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        while let Some(message) = messages_to_blend.next().await {
            handle_messages_to_blend(message, &mut cryptoraphic_processor, backend).await;
        }

        Ok(())
    }
}

/// Blend a new message received from another service.
async fn handle_messages_to_blend<NodeId, Rng, Backend, RuntimeServiceId>(
    message: Vec<u8>,
    cryptographic_processor: &mut CryptographicProcessor<NodeId, Rng>,
    backend: &Backend,
) where
    NodeId: Clone + Send,
    Rng: RngCore + Send,
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Sync,
{
    let Ok(message) = cryptographic_processor
        .encapsulate_data_payload(&message)
        .inspect_err(|e| {
            tracing::error!(target: LOG_TARGET, "Failed to encapsulate message: {e:?}");
        })
    else {
        return;
    };
    backend.send(serialize_encapsulated_message(&message)).await;
}
