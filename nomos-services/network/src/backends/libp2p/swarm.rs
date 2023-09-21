use std::{collections::HashMap, ops::Range, time::Duration};

use mixnet_client::MixnetClient;
#[cfg(feature = "metrics")]
use nomos_libp2p::libp2p::metrics::Recorder;
use nomos_libp2p::{
    gossipsub::{self, Message},
    libp2p::swarm::ConnectionId,
    Behaviour, BehaviourEvent, Multiaddr, Swarm, SwarmEvent, THandlerErr,
};
use rand::rngs::OsRng;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::StreamExt;

use crate::backends::libp2p::{
    mixnet::{random_delay, MixnetMessage},
    Libp2pInfo,
};

use super::{
    command::{Command, Dial, Topic},
    Event, Libp2pConfig,
};

pub struct SwarmHandler {
    pub swarm: Swarm,
    pub pending_dials: HashMap<ConnectionId, Dial>,
    pub commands_tx: mpsc::Sender<Command>,
    pub commands_rx: mpsc::Receiver<Command>,
    pub events_tx: broadcast::Sender<Event>,
    pub mixnet_client: MixnetClient<OsRng>,
    pub mixnet_delay: Range<Duration>,
}

macro_rules! log_error {
    ($e:expr) => {
        if let Err(e) = $e {
            tracing::error!("error while processing {}: {e:?}", stringify!($e));
        }
    };
}

// TODO: make this configurable
const BACKOFF: u64 = 5;
// TODO: make this configurable
const MAX_RETRY: usize = 3;

impl SwarmHandler {
    pub fn new(
        config: &Libp2pConfig,
        commands_tx: mpsc::Sender<Command>,
        commands_rx: mpsc::Receiver<Command>,
        events_tx: broadcast::Sender<Event>,
    ) -> Self {
        let swarm = Swarm::build(&config.inner).unwrap();
        let mixnet_client = MixnetClient::new(config.mixnet_client.clone(), OsRng);

        // Keep the dialing history since swarm.connect doesn't return the result synchronously
        let pending_dials = HashMap::<ConnectionId, Dial>::new();

        Self {
            swarm,
            pending_dials,
            commands_tx,
            commands_rx,
            events_tx,
            mixnet_client,
            mixnet_delay: config.mixnet_delay.clone(),
        }
    }

    pub async fn run(&mut self, initial_peers: Vec<Multiaddr>) {
        for initial_peer in initial_peers {
            let dial = Dial {
                addr: initial_peer,
                retry_count: 0,
            };
            Self::schedule_connect(dial, self.commands_tx.clone()).await;
        }

        loop {
            tokio::select! {
                Some(event) = self.swarm.next() => {
                    self.handle_event(event);
                }
                Some(command) = self.commands_rx.recv() => {
                    self.handle_command(command).await;
                }
            }
        }
    }

    fn handle_event(&mut self, event: SwarmEvent<BehaviourEvent, THandlerErr<Behaviour>>) {
        match event {
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(event)) => {
                self.handle_gossipsub_event(event);
            }
            swarm_event => {
                self.handle_swarm_event(swarm_event);
            }
        }
    }

    fn handle_gossipsub_event(&mut self, event: gossipsub::Event) {
        #[cfg(feature = "metrics")]
        self.swarm.metrics().record(&event);

        match event {
            gossipsub::Event::Message {
                propagation_source,
                message_id,
                message,
            } => {
                tracing::debug!(
                    "Got message with id: {message_id} propagated from {propagation_source}"
                );
                log_error!(self.events_tx.send(Event::Message(message)));
            }
            event => {
                tracing::debug!("gossipsub event: {event:?}");
            }
        }
    }

    fn handle_swarm_event(&mut self, event: SwarmEvent<BehaviourEvent, THandlerErr<Behaviour>>) {
        #[cfg(feature = "metrics")]
        self.swarm.metrics().record(&event);

        match event {
            SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                ..
            } => {
                tracing::debug!("connected to peer:{peer_id}, connection_id:{connection_id:?}");
                if endpoint.is_dialer() {
                    self.complete_connect(connection_id);
                }
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                connection_id,
                cause,
                ..
            } => {
                tracing::debug!(
                    "connection closed from peer: {peer_id} {connection_id:?} due to {cause:?}"
                );
            }
            SwarmEvent::OutgoingConnectionError {
                peer_id,
                connection_id,
                error,
                ..
            } => {
                tracing::error!(
                    "Failed to connect to peer: {peer_id:?} {connection_id:?} due to: {error}"
                );
                self.retry_connect(connection_id);
            }
            event => {
                tracing::debug!("swarm event: {event:?}");
            }
        }
    }

    async fn handle_command(&mut self, command: Command) {
        match command {
            Command::Connect(dial) => {
                self.connect(dial);
            }
            Command::Broadcast { topic, message } => {
                tracing::debug!("sending message to mixnet client");
                let msg = MixnetMessage { topic, message };
                let delay = random_delay(&self.mixnet_delay);
                log_error!(self.mixnet_client.send(msg.as_bytes(), delay));
            }
            Command::Subscribe(topic) => {
                tracing::debug!("subscribing to topic: {topic}");
                log_error!(self.swarm.subscribe(&topic));
            }
            Command::Unsubscribe(topic) => {
                tracing::debug!("unsubscribing to topic: {topic}");
                log_error!(self.swarm.unsubscribe(&topic));
            }
            Command::Info { reply } => {
                let swarm = self.swarm.swarm();
                let network_info = swarm.network_info();
                let counters = network_info.connection_counters();
                let info = Libp2pInfo {
                    listen_addresses: swarm.listeners().cloned().collect(),
                    n_peers: network_info.num_peers(),
                    n_connections: counters.num_connections(),
                    n_pending_connections: counters.num_pending(),
                };
                log_error!(reply.send(info));
            }
            Command::DirectBroadcastAndRetry {
                topic,
                message,
                retry_count,
            } => {
                self.broadcast_and_retry(topic, message, retry_count).await;
            }
        }
    }

    async fn schedule_connect(dial: Dial, commands_tx: mpsc::Sender<Command>) {
        commands_tx
            .send(Command::Connect(dial))
            .await
            .unwrap_or_else(|_| tracing::error!("could not schedule connect"));
    }

    fn connect(&mut self, dial: Dial) {
        tracing::debug!("Connecting to {}", dial.addr);

        match self.swarm.connect(dial.addr.clone()) {
            Ok(connection_id) => {
                // Dialing has been scheduled. The result will be notified as a SwarmEvent.
                self.pending_dials.insert(connection_id, dial);
            }
            Err(e) => {
                tracing::error!(
                    "Failed to connect to {} with unretriable error: {e}",
                    dial.addr
                );
            }
        }
    }

    fn complete_connect(&mut self, connection_id: ConnectionId) {
        self.pending_dials.remove(&connection_id);
    }

    // TODO: Consider a common retry module for all use cases
    fn retry_connect(&mut self, connection_id: ConnectionId) {
        if let Some(mut dial) = self.pending_dials.remove(&connection_id) {
            dial.retry_count += 1;
            if dial.retry_count > MAX_RETRY {
                tracing::debug!("Max retry({MAX_RETRY}) has been reached: {dial:?}");
                return;
            }

            let wait = Self::exp_backoff(dial.retry_count);
            tracing::debug!("Retry dialing in {wait:?}: {dial:?}");

            let commands_tx = self.commands_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(wait).await;
                Self::schedule_connect(dial, commands_tx).await;
            });
        }
    }

    async fn broadcast_and_retry(&mut self, topic: Topic, message: Box<[u8]>, retry_count: usize) {
        tracing::debug!("broadcasting message to topic: {topic}");

        match self.swarm.broadcast(&topic, message.to_vec()) {
            Ok(id) => {
                tracing::debug!("broadcasted message with id: {id} tp topic: {topic}");
                // self-notification because libp2p doesn't do it
                if self.swarm.is_subscribed(&topic) {
                    log_error!(self.events_tx.send(Event::Message(Message {
                        source: None,
                        data: message.into(),
                        sequence_number: None,
                        topic: Swarm::topic_hash(&topic),
                    })));
                }
            }
            Err(gossipsub::PublishError::InsufficientPeers) if retry_count < MAX_RETRY => {
                let wait = Self::exp_backoff(retry_count);
                tracing::error!("failed to broadcast message to topic due to insufficient peers, trying again in {wait:?}");

                let commands_tx = self.commands_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(wait).await;
                    commands_tx
                        .send(Command::DirectBroadcastAndRetry {
                            topic,
                            message,
                            retry_count: retry_count + 1,
                        })
                        .await
                        .unwrap_or_else(|_| tracing::error!("could not schedule retry"));
                });
            }
            Err(e) => {
                tracing::error!("failed to broadcast message to topic: {topic} {e:?}");
            }
        }
    }

    fn exp_backoff(retry: usize) -> Duration {
        std::time::Duration::from_secs(BACKOFF.pow(retry as u32))
    }
}
