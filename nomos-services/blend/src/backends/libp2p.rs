use std::{collections::HashSet, pin::Pin, time::Duration};

use async_trait::async_trait;
use futures::{Stream, StreamExt as _};
use libp2p::{
    allow_block_list::BlockedPeers,
    connection_limits::ConnectionLimits,
    identity::{ed25519, Keypair},
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm, SwarmBuilder,
};
use nomos_blend::{conn_maintenance::ConnectionMonitorSettings, membership::Membership};
use nomos_blend_message::sphinx::SphinxMessage;
use nomos_blend_network::TokioIntervalStreamProvider;
use nomos_libp2p::{secret_key_serde, NetworkBehaviour};
use overwatch::overwatch::handle::OverwatchHandle;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::BroadcastStream;

use super::BlendBackend;

/// A blend backend that uses the libp2p network stack.
pub struct Libp2pBlendBackend {
    swarm_message_sender: mpsc::Sender<BlendSwarmMessage>,
    incoming_message_sender: broadcast::Sender<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Libp2pBlendBackendSettings {
    pub listening_address: Multiaddr,
    // A key for deriving PeerId and establishing secure connections (TLS 1.3 by QUIC)
    #[serde(with = "secret_key_serde", default = "ed25519::SecretKey::generate")]
    pub node_key: ed25519::SecretKey,
    pub peering_degree: u16,
    pub max_peering_degree: u16,
    pub conn_monitor: Option<ConnectionMonitorSettings>,
}

const CHANNEL_SIZE: usize = 64;

#[async_trait]
impl<RuntimeServiceId> BlendBackend<RuntimeServiceId> for Libp2pBlendBackend {
    type Settings = Libp2pBlendBackendSettings;
    type NodeId = PeerId;

    fn new<R>(
        config: Self::Settings,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        membership: Membership<Self::NodeId, SphinxMessage>,
        rng: R,
    ) -> Self
    where
        R: RngCore + Send + 'static,
    {
        let (swarm_message_sender, swarm_message_receiver) = mpsc::channel(CHANNEL_SIZE);
        let (incoming_message_sender, _) = broadcast::channel(CHANNEL_SIZE);

        let mut swarm = BlendSwarm::new(
            config,
            membership,
            rng,
            swarm_message_receiver,
            incoming_message_sender.clone(),
        );

        overwatch_handle.runtime().spawn(async move {
            swarm.run().await;
        });

        Self {
            swarm_message_sender,
            incoming_message_sender,
        }
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

struct BlendSwarm<R> {
    swarm: Swarm<BlendBehaviour>,
    swarm_messages_receiver: mpsc::Receiver<BlendSwarmMessage>,
    incoming_message_sender: broadcast::Sender<Vec<u8>>,
    // TODO: Instead of holding the membership, we just want a way to get the list of addresses.
    membership: Membership<PeerId, SphinxMessage>,
    rng: R,
    peering_degree: u16,
}

#[derive(NetworkBehaviour)]
struct BlendBehaviour {
    blend: nomos_blend_network::Behaviour<SphinxMessage, TokioIntervalStreamProvider>,
    limits: libp2p::connection_limits::Behaviour,
    blocked_peers: libp2p::allow_block_list::Behaviour<BlockedPeers>,
}

impl BlendBehaviour {
    fn new(config: &Libp2pBlendBackendSettings) -> Self {
        Self {
            blend:
                nomos_blend_network::Behaviour::<SphinxMessage, TokioIntervalStreamProvider>::new(
                    nomos_blend_network::Config {
                        duplicate_cache_lifespan: 60,
                        conn_monitor_settings: config.conn_monitor,
                    },
                ),
            limits: libp2p::connection_limits::Behaviour::new(
                ConnectionLimits::default()
                    .with_max_established(Some(config.max_peering_degree.into()))
                    .with_max_established_incoming(Some(config.max_peering_degree.into()))
                    .with_max_established_outgoing(Some(config.max_peering_degree.into()))
                    // Blend protocol restricts the number of connections per peer to 1.
                    .with_max_established_per_peer(Some(1)),
            ),
            blocked_peers: libp2p::allow_block_list::Behaviour::default(),
        }
    }
}

#[derive(Debug)]
pub enum BlendSwarmMessage {
    Publish(Vec<u8>),
}

impl<R> BlendSwarm<R>
where
    R: RngCore,
{
    fn new(
        config: Libp2pBlendBackendSettings,
        membership: Membership<PeerId, SphinxMessage>,
        mut rng: R,
        swarm_messages_receiver: mpsc::Receiver<BlendSwarmMessage>,
        incoming_message_sender: broadcast::Sender<Vec<u8>>,
    ) -> Self {
        let keypair = Keypair::from(ed25519::Keypair::from(config.node_key.clone()));
        let mut swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_behaviour(|_| BlendBehaviour::new(&config))
            .expect("Blend Behaviour should be built")
            .with_swarm_config(|cfg| {
                // The idle timeout starts ticking once there are no active streams on a
                // connection. We want the connection to be closed as soon as
                // all streams are dropped.
                cfg.with_idle_connection_timeout(Duration::ZERO)
            })
            .build();

        swarm
            .listen_on(config.listening_address)
            .unwrap_or_else(|e| {
                panic!("Failed to listen on Blend network: {e:?}");
            });

        // Dial the initial peers randomly selected
        membership
            .choose_remote_nodes(&mut rng, config.peering_degree as usize)
            .iter()
            .for_each(|peer| {
                if let Err(e) = swarm.dial(peer.address.clone()) {
                    tracing::error!("Failed to dial a peer: {e:?}");
                }
            });

        Self {
            swarm,
            swarm_messages_receiver,
            incoming_message_sender,
            membership,
            rng,
            peering_degree: config.peering_degree,
        }
    }

    async fn run(&mut self) {
        loop {
            tokio::select! {
                Some(msg) = self.swarm_messages_receiver.recv() => {
                    self.handle_swarm_message(msg);
                }
                Some(event) = self.swarm.next() => {
                    self.handle_event(event);
                }
            }
        }
    }

    fn handle_swarm_message(&mut self, msg: BlendSwarmMessage) {
        match msg {
            BlendSwarmMessage::Publish(msg) => {
                self.handle_publish_swarm_message(&msg);
            }
        }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: Address this at some point."
    )]
    fn handle_publish_swarm_message(&mut self, msg: &[u8]) {
        let msg_size = msg.len();
        if let Err(e) = self.swarm.behaviour_mut().blend.publish(msg) {
            tracing::error!("Failed to publish message to blend network: {e:?}");
            tracing::info!(counter.failed_outbound_messages = 1);
        } else {
            tracing::info!(counter.successful_outbound_messages = 1);
            tracing::info!(histogram.sent_data = msg_size as u64);
        }
    }

    fn handle_event(&mut self, event: SwarmEvent<BlendBehaviourEvent>) {
        match event {
            SwarmEvent::Behaviour(BlendBehaviourEvent::Blend(e)) => {
                self.handle_blend_behaviour_event(e);
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                connection_id,
                ..
            } => {
                tracing::error!(
                    "Connection closed: peer:{}, conn_id:{}",
                    peer_id,
                    connection_id
                );
                self.check_and_dial_new_peers();
            }
            _ => {
                tracing::debug!("Received event from blend network: {event:?}");
                tracing::info!(counter.ignored_event = 1);
            }
        }
    }

    fn handle_blend_behaviour_event(&mut self, blend_event: nomos_blend_network::Event) {
        match blend_event {
            nomos_blend_network::Event::Message(msg) => {
                self.handle_blend_message(msg);
            }
            nomos_blend_network::Event::MaliciousPeer(peer_id) => {
                self.handle_malicious_peer(peer_id);
            }
            nomos_blend_network::Event::UnhealthyPeer(peer_id) => {
                self.handle_unhealthy_peer(peer_id);
            }
            nomos_blend_network::Event::Error(e) => {
                tracing::error!("Received error from blend network: {e:?}");
                self.check_and_dial_new_peers();
                tracing::info!(counter.error = 1);
            }
        }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: Address this at some point."
    )]
    fn handle_blend_message(&self, msg: Vec<u8>) {
        tracing::debug!("Received message from a peer: {msg:?}");

        let msg_size = msg.len();
        if let Err(e) = self.incoming_message_sender.send(msg) {
            tracing::error!("Failed to send incoming message to channel: {e}");
            tracing::info!(counter.failed_inbound_messages = 1);
        } else {
            tracing::info!(counter.successful_inbound_messages = 1);
            tracing::info!(histogram.received_data = msg_size as u64);
        }
    }

    fn handle_malicious_peer(&mut self, peer_id: PeerId) {
        tracing::debug!("Peer {} is malicious", peer_id);
        self.swarm.behaviour_mut().blocked_peers.block_peer(peer_id);
        self.check_and_dial_new_peers();
    }

    fn handle_unhealthy_peer(&mut self, peer_id: PeerId) {
        tracing::debug!("Peer {} is unhealthy", peer_id);
        self.check_and_dial_new_peers();
    }

    /// Dial new peers, if necessary, to maintain the peering degree.
    /// We aim to have at least the peering degree number of "healthy" peers.
    fn check_and_dial_new_peers(&mut self) {
        let num_new_conns_needed = (self.peering_degree as usize)
            .saturating_sub(self.swarm.behaviour().blend.num_healthy_peers());
        if num_new_conns_needed > 0 {
            self.dial_random_peers(num_new_conns_needed);
        }
    }

    /// Dial random peers from the membership list,
    /// excluding the currently connected peers and the blocked peers.
    fn dial_random_peers(&mut self, amount: usize) {
        let exclude_peers: HashSet<PeerId> = self
            .swarm
            .connected_peers()
            .chain(self.swarm.behaviour().blocked_peers.blocked_peers())
            .copied()
            .collect();
        self.membership
            .filter_and_choose_remote_nodes(&mut self.rng, amount, &exclude_peers)
            .iter()
            .for_each(|peer| {
                if let Err(e) = self.swarm.dial(peer.address.clone()) {
                    tracing::error!("Failed to dial a peer: {e:?}");
                }
            });
    }
}
