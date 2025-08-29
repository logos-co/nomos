pub mod backends;
pub(crate) mod service_components;
pub mod settings;

use std::{fmt::Display, hash::Hash};

use backends::BlendBackend;
use nomos_blend_scheduling::message_blend::crypto::CryptographicProcessor;
use nomos_core::wire;
use nomos_utils::blake_rng::BlakeRng;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use rand::{RngCore, SeedableRng as _};
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
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
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
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    BroadcastSettings: Serialize + Send,
    RuntimeServiceId: AsServiceId<Self> + Display + Clone + Send,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            service_resources_handle,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            service_resources_handle,
        } = self;

        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        let membership = settings.membership();
        let current_membership = Some(membership.clone());
        let minimum_network_size = settings.minimum_network_size;

        // TODO: Add logic to try process new sessions. I.e:
        // * If the old session membership was too small and the new one is large
        //   enough, create a new backend and start blending incoming messages.
        // * If the old and new session membership are both too small, do nothing.
        // * If the old session membership was large and the new one too small, simply
        //   drop the backend.
        // * If the old and new session membership are both large enough, perform
        //   session rotation logic and maintain the swarm backend.
        // Ideally, this service would be stopped altogether by the proxy service when a
        // session is too small. Yet, the service itself must be resilient in case of
        // bugs where the proxy service does not do that, hence the need for this
        // additional logic.
        if membership.size() < minimum_network_size.get() as usize {
            tracing::warn!(target: LOG_TARGET, "Blend network size is smaller than the required minimum. Not starting swarm, hence no messages will be blended in this session.");
            // We still mark the service as ready, albeit other services won't be able to
            // interact with this service by sending messages to it, and it indeed should
            // not happen, as all interactions should happen via the proxy service.
            service_resources_handle.status_updater.notify_ready();
            tracing::info!(
                target: LOG_TARGET,
                "Service '{}' is ready.",
                <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
            );
        } else {
            let mut cryptoraphic_processor = CryptographicProcessor::new(
                settings.crypto.clone(),
                settings.membership(),
                BlakeRng::from_entropy(),
            );
            let mut messages_to_blend =
                service_resources_handle
                    .inbound_relay
                    .map(|ServiceMessage::Blend(message)| {
                        wire::serialize(&message)
                            .expect("Message from internal services should not fail to serialize")
                    });
            let backend = <Backend as BlendBackend<NodeId, RuntimeServiceId>>::new(
                settings.backend,
                service_resources_handle.overwatch_handle.clone(),
                Box::pin(
                    IntervalStream::new(interval(settings.time.session_duration()))
                        .map(move |_| membership.clone()),
                ),
                current_membership,
                BlakeRng::from_entropy(),
            );

            service_resources_handle.status_updater.notify_ready();
            tracing::info!(
                target: LOG_TARGET,
                "Service '{}' is ready.",
                <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
            );

            while let Some(message) = messages_to_blend.next().await {
                handle_messages_to_blend(message, &mut cryptoraphic_processor, &backend).await;
            }
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
    NodeId: Eq + Hash + Clone + Send,
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
    backend.send(message).await;
}
