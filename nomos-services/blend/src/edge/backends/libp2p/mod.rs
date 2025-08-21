mod settings;
mod swarm;

use std::pin::Pin;

use futures::{
    future::{AbortHandle, Abortable},
    Stream,
};
use libp2p::PeerId;
use nomos_blend_scheduling::{membership::Membership, EncapsulatedMessage};
use overwatch::overwatch::OverwatchHandle;
use rand::RngCore;
pub use settings::Libp2pBlendBackendSettings;
use swarm::BlendSwarm;
use tokio::sync::mpsc;

use super::BlendBackend;

const LOG_TARGET: &str = "blend::service::edge::backend::libp2p";

#[cfg(test)]
mod tests;

pub struct Libp2pBlendBackend {
    swarm_task_abort_handle: AbortHandle,
    swarm_command_sender: mpsc::Sender<swarm::Command>,
}

const CHANNEL_SIZE: usize = 64;

#[async_trait::async_trait]
impl<RuntimeServiceId> BlendBackend<PeerId, RuntimeServiceId> for Libp2pBlendBackend {
    type Settings = Libp2pBlendBackendSettings;

    fn new<Rng>(
        settings: Self::Settings,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        session_stream: Pin<Box<dyn Stream<Item = Membership<PeerId>> + Send>>,
        current_membership: Option<Membership<PeerId>>,
        rng: Rng,
    ) -> Self
    where
        Rng: RngCore + Send + 'static,
    {
        let (swarm_command_sender, swarm_command_receiver) = mpsc::channel(CHANNEL_SIZE);
        let swarm = BlendSwarm::new(
            &settings,
            session_stream,
            current_membership,
            rng,
            swarm_command_receiver,
        );

        let (swarm_task_abort_handle, swarm_task_abort_registration) = AbortHandle::new_pair();
        overwatch_handle
            .runtime()
            .spawn(Abortable::new(swarm.run(), swarm_task_abort_registration));

        Self {
            swarm_task_abort_handle,
            swarm_command_sender,
        }
    }

    fn shutdown(&mut self) {
        self.swarm_task_abort_handle.abort();
    }

    async fn send(&self, msg: EncapsulatedMessage) {
        if let Err(e) = self
            .swarm_command_sender
            .send(swarm::Command::SendMessage(msg))
            .await
        {
            tracing::error!(target: LOG_TARGET, "Failed to send command to Swarm: {e}");
        }
    }
}
