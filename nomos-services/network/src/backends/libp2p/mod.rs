mod command;
mod config;
pub(crate) mod swarm;

use nomos_libp2p::cryptarchia_sync;
pub use nomos_libp2p::libp2p::gossipsub::{Message, TopicHash};
use overwatch::overwatch::handle::OverwatchHandle;
use tokio::sync::{broadcast, broadcast::Sender, mpsc};
use tokio_stream::wrappers::BroadcastStream;

use self::swarm::SwarmHandler;
pub use self::{
    command::{Command, Dial, DiscoveryCommand, Libp2pInfo, NetworkCommand, PubSubCommand},
    config::Libp2pConfig,
};
use super::NetworkBackend;

pub struct Libp2p {
    pubsub_events_tx: Sender<Message>,
    chainsync_events_tx: Sender<cryptarchia_sync::Event>,
    commands_tx: mpsc::Sender<Command>,
}
const BUFFER_SIZE: usize = 64;

#[async_trait::async_trait]
impl<RuntimeServiceId> NetworkBackend<RuntimeServiceId> for Libp2p {
    type Settings = Libp2pConfig;
    type Message = Command;
    type PubSubEvent = Message;
    type ChainSyncEvent = cryptarchia_sync::Event;

    fn new(config: Self::Settings, overwatch_handle: OverwatchHandle<RuntimeServiceId>) -> Self {
        let (commands_tx, commands_rx) = mpsc::channel(BUFFER_SIZE);

        let (pubsub_events_tx, _) = broadcast::channel(BUFFER_SIZE);
        let (chainsync_events_tx, _) = broadcast::channel(BUFFER_SIZE);

        let initial_peers = config.initial_peers.clone();

        let mut swarm_handler = SwarmHandler::new(
            config,
            commands_tx.clone(),
            commands_rx,
            pubsub_events_tx.clone(),
            chainsync_events_tx.clone(),
        );

        overwatch_handle.runtime().spawn(async move {
            swarm_handler.run(initial_peers).await;
        });

        Self {
            pubsub_events_tx,
            chainsync_events_tx,
            commands_tx,
        }
    }

    async fn process(&self, msg: Self::Message) {
        if let Err(e) = self.commands_tx.send(msg).await {
            tracing::error!("failed to send command to nomos-libp2p: {e:?}");
        }
    }

    async fn subscribe_to_pubsub(&mut self) -> BroadcastStream<Self::PubSubEvent> {
        BroadcastStream::new(self.pubsub_events_tx.subscribe())
    }

    async fn subscribe_to_chainsync(&mut self) -> BroadcastStream<Self::ChainSyncEvent> {
        BroadcastStream::new(self.chainsync_events_tx.subscribe())
    }
}
