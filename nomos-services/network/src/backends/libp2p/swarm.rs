use std::{collections::HashMap, time::Duration};

use nomos_libp2p::{
    gossipsub,
    libp2p::{
        identify,
        kad::{self, PeerInfo, ProgressStep, QueryId},
        swarm::ConnectionId,
    },
    BehaviourEvent, Multiaddr, PeerId, Protocol, Swarm, SwarmEvent,
};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_stream::StreamExt;

use super::{
    command::{Command, Dial, DiscoveryCommand, NetworkCommand, PubSubCommand, Topic},
    Event, Libp2pConfig,
};
use crate::backends::libp2p::Libp2pInfo;

// Define a struct to hold the data
struct PendingQueryData {
    sender: oneshot::Sender<Vec<PeerInfo>>,
    accumulated_results: Vec<PeerInfo>,
}

pub struct SwarmHandler {
    pub swarm: Swarm,
    pub pending_dials: HashMap<ConnectionId, Dial>,
    pub commands_tx: mpsc::Sender<Command>,
    pub commands_rx: mpsc::Receiver<Command>,
    pub events_tx: broadcast::Sender<Event>,

    pending_queries: HashMap<QueryId, PendingQueryData>,
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
        config: Libp2pConfig,
        commands_tx: mpsc::Sender<Command>,
        commands_rx: mpsc::Receiver<Command>,
        events_tx: broadcast::Sender<Event>,
    ) -> Self {
        let swarm = Swarm::build(config.inner).unwrap();

        // Keep the dialing history since swarm.connect doesn't return the result
        // synchronously
        let pending_dials = HashMap::<ConnectionId, Dial>::new();

        Self {
            swarm,
            pending_dials,
            commands_tx,
            commands_rx,
            events_tx,
            pending_queries: HashMap::new(),
        }
    }

    pub async fn run(&mut self, initial_peers: Vec<Multiaddr>) {
        let local_peer_id = *self.swarm.swarm().local_peer_id();
        let local_addr = self.swarm.swarm().listeners().next().cloned();

        // add local address to kademlia
        if let Some(addr) = local_addr {
            self.swarm
                .swarm_mut()
                .behaviour_mut()
                .kademlia_add_address(local_peer_id, addr);
        }

        self.bootstrap_kad_from_peers(&initial_peers);

        for initial_peer in &initial_peers {
            let (tx, _) = oneshot::channel();
            let dial = Dial {
                addr: initial_peer.clone(),
                retry_count: 0,
                result_sender: tx,
            };
            Self::schedule_connect(dial, self.commands_tx.clone()).await;
        }

        loop {
            tokio::select! {
                Some(event) = self.swarm.next() => {
                    self.handle_event(event);
                }
                Some(command) = self.commands_rx.recv() => {
                    self.handle_command(command);
                }
            }
        }
    }

    fn bootstrap_kad_from_peers(&mut self, initial_peers: &Vec<Multiaddr>) {
        for peer_addr in initial_peers {
            if let Some(Protocol::P2p(peer_id_bytes)) = peer_addr.iter().last() {
                if let Ok(peer_id) = PeerId::from_multihash(peer_id_bytes.into()) {
                    self.swarm
                        .swarm_mut()
                        .behaviour_mut()
                        .kademlia_add_address(peer_id, peer_addr.clone());
                    tracing::debug!("Added peer to Kademlia: {} at {}", peer_id, peer_addr);
                } else {
                    tracing::warn!("Failed to parse peer ID from multiaddr: {}", peer_addr);
                }
            } else {
                tracing::warn!("Multiaddr doesn't contain peer ID: {}", peer_addr);
            }
        }
    }

    fn handle_event(&mut self, event: SwarmEvent<BehaviourEvent>) {
        match event {
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(event)) => {
                self.handle_gossipsub_event(event);
            }
            SwarmEvent::Behaviour(BehaviourEvent::Identify(event)) => {
                self.handle_identify_event(event);
            }

            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(event)) => {
                self.handle_kademlia_event(event);
            }

            SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                ..
            } => {
                tracing::debug!("connected to peer:{peer_id}, connection_id:{connection_id:?}");
                if endpoint.is_dialer() {
                    self.complete_connect(connection_id, peer_id);
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
            _ => {}
        }
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::Network(network_cmd) => self.handle_network_command(network_cmd),
            Command::PubSub(pubsub_cmd) => self.handle_pubsub_command(pubsub_cmd),
            Command::Discovery(discovery_cmd) => self.handle_discovery_command(discovery_cmd),
        }
    }

    fn handle_network_command(&mut self, command: NetworkCommand) {
        match command {
            NetworkCommand::Connect(dial) => {
                self.connect(dial);
            }
            NetworkCommand::Info { reply } => {
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
        }
    }

    fn handle_pubsub_command(&mut self, command: PubSubCommand) {
        match command {
            PubSubCommand::Broadcast { topic, message } => {
                self.broadcast_and_retry(topic, message, 0);
            }
            PubSubCommand::Subscribe(topic) => {
                tracing::debug!("subscribing to topic: {topic}");
                log_error!(self.swarm.subscribe(&topic));
            }
            PubSubCommand::Unsubscribe(topic) => {
                tracing::debug!("unsubscribing to topic: {topic}");
                self.swarm.unsubscribe(&topic);
            }
            PubSubCommand::RetryBroadcast {
                topic,
                message,
                retry_count,
            } => {
                self.broadcast_and_retry(topic, message, retry_count);
            }
        }
    }

    fn handle_discovery_command(&mut self, command: DiscoveryCommand) {
        match command {
            DiscoveryCommand::GetClosestPeers { peer_id, reply } => {
                match self.swarm.get_closest_peers(peer_id) {
                    Ok(query_id) => {
                        tracing::debug!("Pending query ID: {query_id}");
                        self.pending_queries.insert(
                            query_id,
                            PendingQueryData {
                                sender: reply,
                                accumulated_results: Vec::new(),
                            },
                        );
                    }
                    Err(err) => {
                        tracing::warn!(
                            "Failed to get closest peers for peer ID: {peer_id}: {:?}",
                            err
                        );
                        let _ = reply.send(Vec::new());
                    }
                }
            }
            DiscoveryCommand::DumpRoutingTable { reply } => {
                let result = self
                    .swarm
                    .swarm_mut()
                    .behaviour_mut()
                    .kademlia_routing_table_dump();
                tracing::debug!("Routing table dump: {result:?}");
                let _ = reply.send(result);
            }
        }
    }

    async fn schedule_connect(dial: Dial, commands_tx: mpsc::Sender<Command>) {
        commands_tx
            .send(Command::Network(NetworkCommand::Connect(dial)))
            .await
            .unwrap_or_else(|_| tracing::error!("could not schedule connect"));
    }

    fn connect(&mut self, dial: Dial) {
        tracing::debug!("Connecting to {}", dial.addr);

        match self.swarm.connect(&dial.addr) {
            Ok(connection_id) => {
                // Dialing has been scheduled. The result will be notified as a SwarmEvent.
                self.pending_dials.insert(connection_id, dial);
            }
            Err(e) => {
                if let Err(err) = dial.result_sender.send(Err(e)) {
                    tracing::warn!("failed to send the Err result of dialing: {err:?}");
                }
            }
        }
    }

    fn complete_connect(&mut self, connection_id: ConnectionId, peer_id: PeerId) {
        if let Some(dial) = self.pending_dials.remove(&connection_id) {
            if let Err(e) = dial.result_sender.send(Ok(peer_id)) {
                tracing::warn!("failed to send the Ok result of dialing: {e:?}");
            }
        }
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

    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: Address this at some point."
    )]
    fn broadcast_and_retry(&mut self, topic: Topic, message: Box<[u8]>, retry_count: usize) {
        tracing::debug!("broadcasting message to topic: {topic}");

        match self.swarm.broadcast(&topic, message.to_vec()) {
            Ok(id) => {
                tracing::debug!("broadcasted message with id: {id} tp topic: {topic}");
                // self-notification because libp2p doesn't do it
                if self.swarm.is_subscribed(&topic) {
                    log_error!(self.events_tx.send(Event::Message(gossipsub::Message {
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
                        .send(Command::PubSub(PubSubCommand::RetryBroadcast {
                            topic,
                            message,
                            retry_count: retry_count + 1,
                        }))
                        .await
                        .unwrap_or_else(|_| tracing::error!("could not schedule retry"));
                });
            }
            Err(e) => {
                tracing::error!("failed to broadcast message to topic: {topic} {e:?}");
            }
        }
    }

    fn handle_gossipsub_event(&self, event: nomos_libp2p::libp2p::gossipsub::Event) {
        if let nomos_libp2p::libp2p::gossipsub::Event::Message { message, .. } = event {
            let message = Event::Message(message);
            if let Err(e) = self.events_tx.send(message) {
                tracing::error!("Failed to send gossipsub message event: {}", e);
            }
        }
    }

    fn handle_identify_event(&mut self, event: identify::Event) {
        match event {
            identify::Event::Received { peer_id, info, .. } => {
                tracing::debug!(
                    "Identified peer {} with addresses {:?}",
                    peer_id,
                    info.listen_addrs
                );
                let kad_protocol_names = self.swarm.get_kademlia_protocol_names();
                if info
                    .protocols
                    .iter()
                    .any(|p| kad_protocol_names.contains(&p.to_string()))
                {
                    // we need to add the peer to the kademlia routing table
                    // in order to enable peer discovery
                    for addr in &info.listen_addrs {
                        self.swarm
                            .swarm_mut()
                            .behaviour_mut()
                            .kademlia_add_address(peer_id, addr.clone());
                    }
                }
            }
            event => {
                tracing::debug!("Identify event: {:?}", event);
            }
        }
    }

    pub fn handle_kademlia_event(&mut self, event: kad::Event) {
        match event {
            kad::Event::OutboundQueryProgressed {
                id, result, step, ..
            } => {
                self.handle_query_progress(id, result, &step);
            }
            kad::Event::RoutingUpdated {
                peer,
                addresses,
                old_peer,
                is_new_peer,
                ..
            } => {
                log_routing_update(peer, &addresses.into_vec(), old_peer, is_new_peer);
            }
            event => {
                tracing::debug!("Kademlia event: {:?}", event);
            }
        }
    }

    fn handle_query_progress(
        &mut self,
        id: QueryId,
        result: kad::QueryResult,
        step: &ProgressStep,
    ) {
        match result {
            kad::QueryResult::GetClosestPeers(Ok(result)) => {
                if let Some(query_data) = self.pending_queries.get_mut(&id) {
                    query_data.accumulated_results.extend(result.peers);

                    if step.last {
                        if let Some(query_data) = self.pending_queries.remove(&id) {
                            let _ = query_data.sender.send(query_data.accumulated_results);
                        }
                    }
                }
            }
            kad::QueryResult::GetClosestPeers(Err(err)) => {
                tracing::warn!("Failed to find closest peers: {:?}", err);
                // For errors, we should probably just send what we have so far
                if let Some(query_data) = self.pending_queries.remove(&id) {
                    let _ = query_data.sender.send(query_data.accumulated_results);
                }
            }
            _ => {
                tracing::debug!("Handle kademlia query result: {:?}", result);
            }
        }
    }

    const fn exp_backoff(retry: usize) -> Duration {
        std::time::Duration::from_secs(BACKOFF.pow(retry as u32))
    }
}

fn log_routing_update(
    peer: PeerId,
    address: &[Multiaddr],
    old_peer: Option<PeerId>,
    is_new_peer: bool,
) {
    tracing::debug!(
        "Routing table updated: peer: {peer}, address: {address:?}, \
         old_peer: {old_peer:?}, is_new_peer: {is_new_peer}"
    );
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, sync::Once, time::Instant};

    use nomos_libp2p::protocol_name::ProtocolName;
    use tracing_subscriber::EnvFilter;

    use super::*;

    static INIT: Once = Once::new();

    fn init_tracing() {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

        INIT.call_once(|| {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        });
    }

    fn create_swarm_config(port: u16) -> nomos_libp2p::SwarmConfig {
        nomos_libp2p::SwarmConfig {
            host: Ipv4Addr::new(127, 0, 0, 1),
            port,
            node_key: nomos_libp2p::ed25519::SecretKey::generate(),
            gossipsub_config: gossipsub::Config::default(),
            kademlia_config: Some(nomos_libp2p::KademliaSettings::default()),
            identify_config: Some(nomos_libp2p::IdentifySettings::default()),
            protocol_name_env: ProtocolName::Unittest,
        }
    }

    fn create_libp2p_config(initial_peers: Vec<Multiaddr>, port: u16) -> Libp2pConfig {
        Libp2pConfig {
            inner: create_swarm_config(port),
            initial_peers,
        }
    }

    const NODE_COUNT: usize = 10;

    #[tokio::test]
    async fn test_kademlia_bootstrap() {
        init_tracing();

        let mut handler_tasks = Vec::with_capacity(NODE_COUNT);
        let mut txs = Vec::new();

        // Create first node (bootstrap node)
        let (tx1, rx1) = mpsc::channel(10);
        txs.push(tx1.clone());

        let (events_tx1, _) = broadcast::channel(10);
        let config = create_libp2p_config(vec![], 8000);
        let mut bootstrap_node = SwarmHandler::new(config, tx1.clone(), rx1, events_tx1.clone());

        let bootstrap_node_peer_id = *bootstrap_node.swarm.swarm().local_peer_id();

        let task1 = tokio::spawn(async move {
            bootstrap_node.run(vec![]).await;
        });
        handler_tasks.push(task1);

        // Wait for bootstrap node to start
        tokio::time::sleep(Duration::from_secs(5)).await;

        let (reply, info_rx) = oneshot::channel();
        tx1.send(Command::Network(NetworkCommand::Info { reply }))
            .await
            .expect("Failed to send info command");
        let bootstrap_info = info_rx.await.expect("Failed to get bootstrap node info");

        assert!(
            !bootstrap_info.listen_addresses.is_empty(),
            "Bootstrap node has no listening addresses"
        );

        tracing::info!(
            "Bootstrap node listening on: {:?}",
            bootstrap_info.listen_addresses
        );

        // Use the first listening address as the bootstrap address
        let bootstrap_addr = bootstrap_info.listen_addresses[0]
            .clone()
            .with(Protocol::P2p(bootstrap_node_peer_id));

        tracing::info!("Using bootstrap address: {}", bootstrap_addr);

        let bootstrap_addr = bootstrap_addr.clone();

        // Start additional nodes
        for i in 1..NODE_COUNT {
            let (tx, rx) = mpsc::channel(10);
            txs.push(tx.clone());
            let (events_tx, _) = broadcast::channel(10);

            // Each node connects to the bootstrap node
            let config = create_libp2p_config(vec![bootstrap_addr.clone()], 8000 + i as u16);
            let mut handler = SwarmHandler::new(config, tx.clone(), rx, events_tx);

            let peer_id = *handler.swarm.swarm().local_peer_id();
            tracing::info!("Starting node {} with peer ID: {}", i, peer_id);

            let bootstrap_addr = bootstrap_addr.clone();
            let task = tokio::spawn(async move {
                handler.run(vec![bootstrap_addr.clone()]).await;
            });

            handler_tasks.push(task);

            // Add small delay between node startups to avoid overloading
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let timeout = Duration::from_secs(30);
        let poll_interval = Duration::from_secs(1);
        let start_time = Instant::now();

        while !txs.is_empty() && start_time.elapsed() < timeout {
            tokio::time::sleep(poll_interval).await;
            let mut indices_to_remove = Vec::new();

            for (idx, tx) in txs.iter().enumerate() {
                let (reply, dump_rx) = oneshot::channel();
                tx.send(Command::Discovery(DiscoveryCommand::DumpRoutingTable {
                    reply,
                }))
                .await
                .expect("Failed to send dump command");

                let routing_table = dump_rx.await.expect("Failed to receive routing table dump");

                let routing_table = routing_table
                    .into_iter()
                    .flat_map(|(_, peers)| peers)
                    .collect::<Vec<_>>();

                if routing_table.len() >= NODE_COUNT - 1 {
                    // This node's routing table is fully populated, mark for removal
                    indices_to_remove.push(idx);
                    tracing::info!(
                        "Node has complete routing table with {} entries",
                        routing_table.len()
                    );
                }
            }

            for idx in indices_to_remove.iter().rev() {
                txs.remove(*idx);
            }
        }

        assert!(
            txs.is_empty(),
            "Timed out after {:?} - {} nodes still have incomplete routing tables",
            timeout,
            txs.len()
        );

        // Verify closest peers from the bootstrap node
        let (closest_tx, closest_rx) = oneshot::channel();
        tx1.send(Command::Discovery(DiscoveryCommand::GetClosestPeers {
            peer_id: bootstrap_node_peer_id,
            reply: closest_tx,
        }))
        .await
        .expect("Failed to send get closest peers command");

        let closest_peers = closest_rx.await.expect("Failed to get closest peers");

        assert!(
            closest_peers.len() >= NODE_COUNT - 1,
            "Expected at least {} closest peers, got {}",
            NODE_COUNT - 1,
            closest_peers.len()
        );

        for task in handler_tasks {
            task.abort();
        }
    }
}
