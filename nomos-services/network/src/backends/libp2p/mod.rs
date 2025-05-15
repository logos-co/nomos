mod command;
mod config;
pub(crate) mod swarm;

pub use nomos_libp2p::libp2p::gossipsub::{Message, TopicHash};
use overwatch::{overwatch::handle::OverwatchHandle, services::state::NoState};
use tokio::sync::{broadcast, mpsc};

use self::swarm::SwarmHandler;
pub use self::{
    command::{Command, Dial, DiscoveryCommand, Libp2pInfo, NetworkCommand, PubSubCommand},
    config::Libp2pConfig,
};
use super::NetworkBackend;

pub struct Libp2p {
    events_tx: broadcast::Sender<Event>,
    commands_tx: mpsc::Sender<Command>,
}

#[derive(Debug)]
pub enum EventKind {
    Message,
}

/// Events emitted from [`NomosLibp2p`], which users can subscribe
#[derive(Debug, Clone)]
pub enum Event {
    Message(Message),
}

const BUFFER_SIZE: usize = 64;

#[async_trait::async_trait]
impl<RuntimeServiceId> NetworkBackend<RuntimeServiceId> for Libp2p {
    type Settings = Libp2pConfig;
    type State = NoState<Libp2pConfig>;
    type Message = Command;
    type EventKind = EventKind;
    type NetworkEvent = Event;

    fn new(config: Self::Settings, overwatch_handle: OverwatchHandle<RuntimeServiceId>) -> Self {
        let (commands_tx, commands_rx) = tokio::sync::mpsc::channel(BUFFER_SIZE);
        let (events_tx, _) = tokio::sync::broadcast::channel(BUFFER_SIZE);
        let initial_peers = config.initial_peers.clone();

        let mut swarm_handler =
            SwarmHandler::new(config, commands_tx.clone(), commands_rx, events_tx.clone());

        overwatch_handle.runtime().spawn(async move {
            swarm_handler.run(initial_peers).await;
        });

        Self {
            events_tx,
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
            EventKind::Message => {
                tracing::debug!("processed subscription to incoming messages");
                self.events_tx.subscribe()
            }
        }
    }
}
