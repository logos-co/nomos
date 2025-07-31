use std::{collections::HashSet, time::Duration};

use futures::{Stream, StreamExt as _};
use libp2p::{identity::Keypair, PeerId, Swarm, SwarmBuilder};
use nomos_blend_network::core::{
    with_core::behaviour::{Event as CoreToCoreEvent, NegotiatedPeerState},
    with_edge::behaviour::Event as CoreToEdgeEvent,
    NetworkBehaviourEvent,
};
use nomos_blend_scheduling::membership::Membership;
use nomos_libp2p::{ed25519, SwarmEvent};
use rand::RngCore;
use tokio::sync::{broadcast, mpsc};

use crate::core::{
    backends::libp2p::{
        behaviour::{BlendBehaviour, BlendBehaviourEvent},
        Libp2pBlendBackendSettings, LOG_TARGET,
    },
    BlendConfig,
};

#[derive(Debug)]
pub enum BlendSwarmMessage {
    Publish(Vec<u8>),
}

pub(super) struct BlendSwarm<SessionStream, Rng> {
    swarm: Swarm<BlendBehaviour>,
    swarm_messages_receiver: mpsc::Receiver<BlendSwarmMessage>,
    incoming_message_sender: broadcast::Sender<Vec<u8>>,
    session_stream: SessionStream,
    latest_session_info: Membership<PeerId>,
    rng: Rng,
    min_outgoing_peering_degree: usize,
}

impl<SessionStream, Rng> BlendSwarm<SessionStream, Rng>
where
    Rng: RngCore,
{
    pub(super) fn new(
        config: BlendConfig<Libp2pBlendBackendSettings, PeerId>,
        session_stream: SessionStream,
        mut rng: Rng,
        swarm_messages_receiver: mpsc::Receiver<BlendSwarmMessage>,
        incoming_message_sender: broadcast::Sender<Vec<u8>>,
    ) -> Self {
        let membership = config.membership();
        let keypair = Keypair::from(ed25519::Keypair::from(config.backend.node_key.clone()));
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
            .listen_on(config.backend.listening_address)
            .unwrap_or_else(|e| {
                panic!("Failed to listen on Blend network: {e:?}");
            });

        let min_outgoing_peering_degree = *config.backend.peering_degree.start() as usize;

        // Dial the initial peers randomly selected
        membership
            .choose_remote_nodes(&mut rng, min_outgoing_peering_degree)
            .for_each(|peer| {
                if let Err(e) = swarm.dial(peer.address.clone()) {
                    tracing::error!(target: LOG_TARGET, "Failed to dial a peer: {e:?}");
                }
            });

        Self {
            swarm,
            swarm_messages_receiver,
            incoming_message_sender,
            session_stream,
            latest_session_info: membership,
            rng,
            min_outgoing_peering_degree,
        }
    }
}

impl<SessionStream, Rng> BlendSwarm<SessionStream, Rng> {
    fn handle_swarm_message(&mut self, msg: BlendSwarmMessage) {
        match msg {
            BlendSwarmMessage::Publish(msg) => {
                self.handle_publish_swarm_message(&msg);
            }
        }
    }

    fn handle_publish_swarm_message(&mut self, msg: &[u8]) {
        self.publish_swarm_message(msg);
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "Tracing macros generate more code that triggers this warning."
    )]
    fn publish_swarm_message(&mut self, msg: &[u8]) {
        if let Err(e) = self
            .swarm
            .behaviour_mut()
            .blend
            .with_core_mut()
            .publish(msg)
        {
            tracing::error!(target: LOG_TARGET, "Failed to publish message to blend network: {e:?}");
            tracing::info!(counter.failed_outbound_messages = 1);
        } else {
            tracing::info!(counter.successful_outbound_messages = 1);
            tracing::info!(histogram.sent_data = msg.len() as u64);
        }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "Tracing macros generate more code that triggers this warning."
    )]
    fn forward_swarm_message(&mut self, msg: &[u8], except: PeerId) {
        if let Err(e) = self
            .swarm
            .behaviour_mut()
            .blend
            .with_core_mut()
            .forward_message(msg, except)
        {
            tracing::error!(target: LOG_TARGET, "Failed to forward message to blend network: {e:?}");
            tracing::info!(counter.failed_outbound_messages = 1);
        } else {
            tracing::info!(counter.successful_outbound_messages = 1);
            tracing::info!(histogram.sent_data = msg.len() as u64);
        }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "Tracing macros generate more code that triggers this warning."
    )]
    fn report_message_to_service(&self, msg: Vec<u8>) {
        tracing::debug!("Received message from a peer: {msg:?}");

        let msg_size = msg.len();
        if let Err(e) = self.incoming_message_sender.send(msg) {
            tracing::error!(target: LOG_TARGET, "Failed to send incoming message to channel: {e}");
            tracing::info!(counter.failed_inbound_messages = 1);
        } else {
            tracing::info!(counter.successful_inbound_messages = 1);
            tracing::info!(histogram.received_data = msg_size as u64);
        }
    }
}

impl<SessionStream, Rng> BlendSwarm<SessionStream, Rng>
where
    Rng: RngCore,
    SessionStream: Stream<Item = Membership<PeerId>> + Unpin,
{
    pub(super) async fn run(mut self) {
        loop {
            tokio::select! {
                Some(msg) = self.swarm_messages_receiver.recv() => {
                    self.handle_swarm_message(msg);
                }
                Some(event) = self.swarm.next() => {
                    self.handle_event(event);
                }
                Some(new_session_info) = self.session_stream.next() => {
                    self.latest_session_info = new_session_info;
                    // TODO: Perform the session transition logic
                }
            }
        }
    }

    fn handle_blend_core_behaviour_event(&mut self, blend_event: CoreToCoreEvent) {
        match blend_event {
            nomos_blend_network::core::with_core::behaviour::Event::Message(msg, peer_id) => {
                // Forward message received from node to all other core nodes.
                self.forward_swarm_message(&msg, peer_id);
                // Report the message to the Blend service for further processing.
                self.report_message_to_service(msg);
            }
            nomos_blend_network::core::with_core::behaviour::Event::UnhealthyPeer(peer_id, _) => {
                self.handle_unhealthy_peer(peer_id);
            }
            nomos_blend_network::core::with_core::behaviour::Event::HealthyPeer(peer_id, _) => {
                Self::handle_healthy_peer(peer_id);
            }
            nomos_blend_network::core::with_core::behaviour::Event::PeerDisconnected(
                peer_id,
                _,
                peer_state,
            ) => {
                self.handle_disconnected_peer(peer_id, peer_state);
            }
        }
    }

    fn handle_blend_edge_behaviour_event(&mut self, blend_event: CoreToEdgeEvent) {
        match blend_event {
            nomos_blend_network::core::with_edge::behaviour::Event::Message(msg) => {
                // Forward message received from edge node to all core nodes.
                self.publish_swarm_message(&msg);
                // Report the message to the Blend service for further processing.
                self.report_message_to_service(msg);
            }
        }
    }

    fn handle_event(&mut self, event: SwarmEvent<BlendBehaviourEvent>) {
        match event {
            SwarmEvent::Behaviour(BlendBehaviourEvent::Blend(NetworkBehaviourEvent::WithCore(
                e,
            ))) => {
                self.handle_blend_core_behaviour_event(e);
            }
            SwarmEvent::Behaviour(BlendBehaviourEvent::Blend(NetworkBehaviourEvent::WithEdge(
                e,
            ))) => {
                self.handle_blend_edge_behaviour_event(e);
            }
            _ => {
                tracing::debug!(target: LOG_TARGET, "Received event from blend network: {event:?}");
                tracing::info!(counter.ignored_event = 1);
            }
        }
    }

    fn handle_unhealthy_peer(&mut self, peer_id: PeerId) {
        tracing::debug!(target: LOG_TARGET, "Peer {peer_id} is unhealthy");
        self.try_dial_new_peer();
    }

    fn handle_healthy_peer(peer_id: PeerId) {
        tracing::debug!(target: LOG_TARGET, "Peer {peer_id} is healthy");
    }

    // If a peer disconnects, we add it to a black-list if it was a spammy one.
    // Then, if the number of healthy outgoing connections drops below the minimum,
    // we try to open a new one, if there is an outgoing slot available.
    fn handle_disconnected_peer(&mut self, peer_id: PeerId, peer_state: NegotiatedPeerState) {
        if peer_state == NegotiatedPeerState::Spammy {
            self.swarm.behaviour_mut().blocked_peers.block_peer(peer_id);
        }
        let is_outgoing_peering_degree_sufficient = self
            .swarm
            .behaviour()
            .blend
            .with_core()
            .healthy_outgoing_connections()
            >= self.min_outgoing_peering_degree;
        if !is_outgoing_peering_degree_sufficient {
            self.try_dial_new_peer();
        }
    }

    /// Dial a new peer, if the underlying swarm has a slot available for a new
    /// connection.
    fn try_dial_new_peer(&mut self) {
        if self
            .swarm
            .behaviour()
            .blend
            .with_core()
            .can_open_outgoing_connection()
        {
            self.dial_random_peers(1);
        } else {
            tracing::warn!("Maximum connections already established. Not possible to open a new outgoing connection.");
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
        self.latest_session_info
            .filter_and_choose_remote_nodes(&mut self.rng, amount, &exclude_peers)
            .iter()
            .for_each(|peer| {
                if let Err(e) = self.swarm.dial(peer.address.clone()) {
                    tracing::error!(target: LOG_TARGET, "Failed to dial a peer: {e:?}");
                }
            });
    }
}
