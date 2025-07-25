mod settings;
mod swarm;

use std::pin::Pin;

use futures::{
    future::{AbortHandle, Abortable},
    Stream,
};
use libp2p::PeerId;
use nomos_blend_scheduling::membership::Membership;
use overwatch::overwatch::OverwatchHandle;
use rand::RngCore;
use settings::Libp2pBlendEdgeBackendSettings;
use swarm::BlendEdgeSwarm;
use tokio::sync::mpsc;

use super::BlendEdgeBackend;

const LOG_TARGET: &str = "blend::service::edge::backend::libp2p";

pub struct Libp2pBlendEdgeBackend {
    swarm_task_abort_handle: AbortHandle,
    swarm_command_sender: mpsc::Sender<swarm::Command>,
}

const CHANNEL_SIZE: usize = 64;

#[async_trait::async_trait]
impl<RuntimeServiceId> BlendEdgeBackend<PeerId, RuntimeServiceId> for Libp2pBlendEdgeBackend {
    type Settings = Libp2pBlendEdgeBackendSettings;

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
        let swarm = BlendEdgeSwarm::new(
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

    async fn send(&self, msg: Vec<u8>) {
        if let Err(e) = self
            .swarm_command_sender
            .send(swarm::Command::SendMessage(msg))
            .await
        {
            tracing::error!(target: LOG_TARGET, "Failed to send command to Swarm: {e}");
        }
    }
}
