use std::pin::Pin;

use async_trait::async_trait;
use futures::{
    future::{AbortHandle, Abortable},
    Stream, StreamExt as _,
};
use libp2p::{identity::ed25519, Multiaddr, PeerId};
use nomos_blend::membership::Membership;
use nomos_libp2p::secret_key_serde;
use overwatch::overwatch::handle::OverwatchHandle;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::BroadcastStream;

use crate::{
    backends::{
        libp2p::swarm::{BlendSwarm, BlendSwarmMessage},
        BlendBackend,
    },
    BlendConfig,
};

mod behaviour;
mod swarm;

/// A blend backend that uses the libp2p network stack.
pub struct Libp2pBlendBackend {
    swarm_task_abort_handle: AbortHandle,
    swarm_message_sender: mpsc::Sender<BlendSwarmMessage>,
    incoming_message_sender: broadcast::Sender<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Libp2pBlendBackendSettings {
    pub listening_address: Multiaddr,
    // A key for deriving PeerId and establishing secure connections (TLS 1.3 by QUIC)
    #[serde(with = "secret_key_serde", default = "ed25519::SecretKey::generate")]
    pub node_key: ed25519::SecretKey,
    pub peering_degree: usize,
    pub max_peering_degree: u32,
}

const CHANNEL_SIZE: usize = 64;

#[async_trait]
impl<RuntimeServiceId> BlendBackend<RuntimeServiceId> for Libp2pBlendBackend {
    type Settings = Libp2pBlendBackendSettings;
    type NodeId = PeerId;

    fn new<Rng>(
        config: BlendConfig<Self::Settings, Self::NodeId>,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        membership: Membership<Self::NodeId>,
        rng: Rng,
    ) -> Self
    where
        Rng: RngCore + Send + 'static,
    {
        let (swarm_message_sender, swarm_message_receiver) = mpsc::channel(CHANNEL_SIZE);
        let (incoming_message_sender, _) = broadcast::channel(CHANNEL_SIZE);

        let swarm = BlendSwarm::new(
            config,
            membership,
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

    async fn publish(&self, msg: Vec<u8>) {
        if let Err(e) = self
            .swarm_message_sender
            .send(BlendSwarmMessage::Publish(msg))
            .await
        {
            tracing::error!("Failed to send message to BlendSwarm: {e}");
        }
    }

    fn listen_to_incoming_messages(&mut self) -> Pin<Box<dyn Stream<Item = Vec<u8>> + Send>> {
        Box::pin(
            BroadcastStream::new(self.incoming_message_sender.subscribe())
                .filter_map(|event| async { event.ok() }),
        )
    }
}
