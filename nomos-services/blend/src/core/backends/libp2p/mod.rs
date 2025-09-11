use std::pin::Pin;

use async_trait::async_trait;
use futures::{
    future::{AbortHandle, Abortable},
    Stream, StreamExt as _,
};
use libp2p::PeerId;
use nomos_blend_network::EncapsulatedMessageWithValidatedPublicHeader;
use nomos_blend_scheduling::{membership::Membership, session::SessionEvent, EncapsulatedMessage};
use overwatch::overwatch::handle::OverwatchHandle;
use rand::RngCore;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::BroadcastStream;

use crate::core::{
    backends::{
        libp2p::{
            swarm::{BlendSwarm, BlendSwarmMessage},
            tokio_provider::ObservationWindowTokioIntervalProvider,
        },
        BlendBackend,
    },
    settings::BlendConfig,
};

const LOG_TARGET: &str = "blend::backend::libp2p";

pub(crate) mod behaviour;
pub mod settings;
pub use self::settings::Libp2pBlendBackendSettings;
mod swarm;
pub(crate) mod tokio_provider;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use self::tests::utils as core_swarm_test_utils;

/// A blend backend that uses the libp2p network stack.
pub struct Libp2pBlendBackend {
    swarm_task_abort_handle: AbortHandle,
    swarm_message_sender: mpsc::Sender<BlendSwarmMessage>,
    incoming_message_sender: broadcast::Sender<EncapsulatedMessageWithValidatedPublicHeader>,
}

const CHANNEL_SIZE: usize = 64;

#[async_trait]
impl<Rng, RuntimeServiceId> BlendBackend<PeerId, Rng, RuntimeServiceId> for Libp2pBlendBackend
where
    Rng: RngCore + Clone + Send + 'static,
{
    type Settings = Libp2pBlendBackendSettings;

    fn new(
        config: BlendConfig<Self::Settings, PeerId>,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        current_membership: Membership<PeerId>,
        session_stream: Pin<Box<dyn Stream<Item = SessionEvent<Membership<PeerId>>> + Send>>,
        rng: Rng,
    ) -> Self {
        let (swarm_message_sender, swarm_message_receiver) = mpsc::channel(CHANNEL_SIZE);
        let (incoming_message_sender, _) = broadcast::channel(CHANNEL_SIZE);
        let minimum_network_size = config.minimum_network_size.try_into().unwrap();

        let swarm = BlendSwarm::<_, _, ObservationWindowTokioIntervalProvider>::new(
            config,
            current_membership,
            session_stream,
            rng,
            swarm_message_receiver,
            incoming_message_sender.clone(),
            minimum_network_size,
        );

        let (swarm_task_abort_handle, swarm_task_abort_registration) = AbortHandle::new_pair();
        overwatch_handle
            .runtime()
            .spawn(Abortable::new(swarm.run(), swarm_task_abort_registration));

        Self {
            swarm_task_abort_handle,
            swarm_message_sender,
            incoming_message_sender,
        }
    }

    fn shutdown(self) {
        drop(self);
    }

    async fn publish(&self, msg: EncapsulatedMessage) {
        if let Err(e) = self
            .swarm_message_sender
            .send(BlendSwarmMessage::Publish(msg))
            .await
        {
            tracing::error!(target: LOG_TARGET, "Failed to send message to BlendSwarm: {e}");
        }
    }

    fn listen_to_incoming_messages(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = EncapsulatedMessageWithValidatedPublicHeader> + Send>> {
        Box::pin(
            BroadcastStream::new(self.incoming_message_sender.subscribe())
                .filter_map(|event| async { event.ok() }),
        )
    }
}

impl Drop for Libp2pBlendBackend {
    fn drop(&mut self) {
        let Self {
            swarm_task_abort_handle,
            ..
        } = self;
        swarm_task_abort_handle.abort();
    }
}
