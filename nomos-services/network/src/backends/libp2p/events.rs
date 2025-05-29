use tokio::sync::{broadcast, broadcast::Sender};

use crate::backends::libp2p::{Event, BUFFER_SIZE};

#[derive(Clone)]
pub struct EventChannels {
    pub message_tx: Sender<Event>,
    pub chainsync_tx: Sender<Event>,
}

impl EventChannels {
    pub fn new() -> Self {
        let (message_tx, _) = broadcast::channel(BUFFER_SIZE);
        let (chainsync_tx, _) = broadcast::channel(BUFFER_SIZE);
        Self {
            message_tx,
            chainsync_tx,
        }
    }

    pub fn subscribe_to_gossipsub(&self) -> broadcast::Receiver<Event> {
        self.message_tx.subscribe()
    }

    pub fn subscribe_to_chainsync(&self) -> broadcast::Receiver<Event> {
        self.chainsync_tx.subscribe()
    }

    pub fn send_gossipsub_message(&self, event: Event) {
        if let Err(e) = self.message_tx.send(event) {
            tracing::error!("failed to send gossipsub message: {e:?}");
        }
    }

    pub fn send_chainsync_event(&self, event: Event) {
        if let Err(e) = self.chainsync_tx.send(event) {
            tracing::error!("failed to send chainsync event: {e:?}");
        }
    }
}
