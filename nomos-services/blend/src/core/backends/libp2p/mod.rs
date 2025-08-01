use std::pin::Pin;

use async_trait::async_trait;
use futures::{
    future::{AbortHandle, Abortable},
    Stream, StreamExt as _,
};
use libp2p::PeerId;
use nomos_blend_scheduling::{membership::Membership, EncapsulatedMessage, UnwrappedMessage};
use overwatch::overwatch::handle::OverwatchHandle;
use rand::RngCore;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::BroadcastStream;

use crate::core::{
    backends::{
        libp2p::swarm::{BlendSwarm, BlendSwarmMessage},
        BlendBackend,
    },
    settings::BlendConfig,
};

const LOG_TARGET: &str = "blend::backend::libp2p";

mod behaviour;
pub mod settings;
pub use settings::Libp2pBlendBackendSettings;
mod swarm;

/// A blend backend that uses the libp2p network stack.
pub struct Libp2pBlendBackend {
    swarm_task_abort_handle: AbortHandle,
    swarm_message_sender: mpsc::Sender<BlendSwarmMessage>,
    incoming_message_sender: broadcast::Sender<UnwrappedMessage>,
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
        session_stream: Pin<Box<dyn Stream<Item = Membership<PeerId>> + Send>>,
        rng: Rng,
    ) -> Self {
        let (swarm_message_sender, swarm_message_receiver) = mpsc::channel(CHANNEL_SIZE);
        let (incoming_message_sender, _) = broadcast::channel(CHANNEL_SIZE);

        let swarm = BlendSwarm::new(
            config,
            session_stream,
            rng,
            swarm_message_receiver,
            incoming_message_sender.clone(),
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

    fn shutdown(&mut self) {
        let Self {
            swarm_task_abort_handle,
            ..
        } = self;
        swarm_task_abort_handle.abort();
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
    ) -> Pin<Box<dyn Stream<Item = UnwrappedMessage> + Send>> {
        Box::pin(
            BroadcastStream::new(self.incoming_message_sender.subscribe())
                .filter_map(|event| async { event.ok() }),
        )
    }
}
