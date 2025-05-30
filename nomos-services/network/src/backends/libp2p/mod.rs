mod command;
mod config;
mod events;
pub(crate) mod swarm;

pub use nomos_libp2p::libp2p::gossipsub::{Message, TopicHash};
use overwatch::overwatch::handle::OverwatchHandle;
use tokio::sync::{broadcast, mpsc};

use self::swarm::SwarmHandler;
pub use self::{
    command::{Command, Dial, DiscoveryCommand, Libp2pInfo, NetworkCommand, PubSubCommand},
    config::Libp2pConfig,
};
use super::NetworkBackend;
use crate::backends::libp2p::{events::EventChannels, swarm::ChainSyncEvent};

pub struct Libp2p {
    event_channels: EventChannels,
    commands_tx: mpsc::Sender<Command>,
}

#[derive(Debug, Clone)]
pub enum EventKind {
    Message,
    ChainSync,
}

/// Events emitted from [`NomosLibp2p`], which users can subscribe
#[derive(Debug, Clone)]
pub enum Event {
    Message(Message),
    ChainSync(ChainSyncEvent),
}

const BUFFER_SIZE: usize = 64;

#[async_trait::async_trait]
impl<RuntimeServiceId> NetworkBackend<RuntimeServiceId> for Libp2p {
    type Settings = Libp2pConfig;
    type Message = Command;
    type EventKind = EventKind;
    type NetworkEvent = Event;

    fn new(config: Self::Settings, overwatch_handle: OverwatchHandle<RuntimeServiceId>) -> Self {
        let (commands_tx, commands_rx) = tokio::sync::mpsc::channel(BUFFER_SIZE);

        let event_channels = EventChannels::new();

        let initial_peers = config.initial_peers.clone();

        let mut swarm_handler = SwarmHandler::new(
            config,
            commands_tx.clone(),
            commands_rx,
            event_channels.clone(),
        );

        overwatch_handle.runtime().spawn(async move {
            swarm_handler.run(initial_peers).await;
        });

        Self {
            event_channels,
            commands_tx,
        }
    }

    async fn process(&self, msg: Self::Message) {
        if let Err(e) = self.commands_tx.send(msg).await {
            tracing::error!("failed to send command to nomos-libp2p: {e:?}");
        }
    }

    async fn subscribe(
        &mut self,
        kind: Self::EventKind,
    ) -> broadcast::Receiver<Self::NetworkEvent> {
        match kind {
            EventKind::Message => self.event_channels.subscribe_to_gossipsub(),
            EventKind::ChainSync => self.event_channels.subscribe_to_chainsync(),
        }
    }
}
