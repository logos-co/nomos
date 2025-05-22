#![allow(
    clippy::multiple_inherent_impl,
    reason = "We spilt the impl in different blocks on purpose to ease localizing changes."
)]

// This macro must be on top if it is accessed by child modules, else if the
// modules are defined before it, they will fail to see it.
macro_rules! log_error {
    ($e:expr) => {
        if let Err(e) = $e {
            tracing::error!("error while processing {}: {e:?}", stringify!($e));
        }
    };
}

use std::time::Duration;

use futures::StreamExt as _;
use nomos_libp2p::{
    swarm::{behaviour::NomosP2pBehaviourEvent, DialRequest, Swarm},
    Multiaddr, SwarmEvent,
};
use tokio::sync::{broadcast, mpsc};

use super::{
    command::{Command, NetworkCommand},
    Event, Libp2pConfig,
};
use crate::backends::libp2p::Libp2pInfo;

mod gossipsub;
mod kademlia;

pub use gossipsub::PubSubCommand;
pub use kademlia::DiscoveryCommand;

pub struct SwarmHandler {
    pub swarm: Swarm,
    pub commands_tx: mpsc::Sender<Command>,
    pub commands_rx: mpsc::Receiver<Command>,
    pub events_tx: broadcast::Sender<Event>,
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

        Self {
            swarm,
            commands_tx,
            commands_rx,
            events_tx,
        }
    }

    pub async fn run(&mut self, initial_peers: Vec<Multiaddr>) {
        self.swarm.start(&initial_peers);

        loop {
            tokio::select! {
                Some(command) = self.commands_rx.recv() => {
                    self.handle_command(command);
                }
                Some(swarm_event) = self.swarm.next() => {
                    self.swarm.handle_event(&swarm_event);
                    self.handle_swarm_event(swarm_event);
                }
            }
        }
    }

    fn handle_swarm_event(&self, event: SwarmEvent<NomosP2pBehaviourEvent>) {
        if let SwarmEvent::Behaviour(NomosP2pBehaviourEvent::Gossipsub(event)) = event {
            self.handle_gossipsub_event(event);
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
                log_error!(self.swarm.schedule_connect(DialRequest {
                    addr: dial.addr,
                    result_sender: dial.result_sender
                }));
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
}

const fn exp_backoff(retry: usize) -> Duration {
    Duration::from_secs(BACKOFF.pow(retry as u32))
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, sync::Once, time::Instant};

    use nomos_libp2p::{
        swarm::{config::SwarmConfig, protocol::ProtocolName},
        Protocol,
    };
    use tokio::sync::oneshot;
    use tracing_subscriber::EnvFilter;

    use super::*;

    static INIT: Once = Once::new();

    fn init_tracing() {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

        INIT.call_once(|| {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        });
    }

    fn create_swarm_config(port: u16) -> SwarmConfig {
        SwarmConfig {
            host: Ipv4Addr::new(127, 0, 0, 1),
            port,
            node_key: nomos_libp2p::ed25519::SecretKey::generate(),
            gossipsub_config: nomos_libp2p::gossipsub::Config::default(),
            kademlia_config: Some(
                nomos_libp2p::swarm::behaviour::kademlia::settings::Settings::default(),
            ),
            identify_config: Some(
                nomos_libp2p::swarm::behaviour::identify::settings::Settings::default(),
            ),
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
