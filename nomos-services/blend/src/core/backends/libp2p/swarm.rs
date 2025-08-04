use std::{collections::HashSet, time::Duration};

use futures::{Stream, StreamExt as _};
use libp2p::{identity::Keypair, PeerId, Swarm, SwarmBuilder};
use nomos_blend_scheduling::{membership::Membership, EncapsulatedMessage, UnwrappedMessage};
use nomos_libp2p::{ed25519, SwarmEvent};
use rand::RngCore;
use tokio::sync::{broadcast, mpsc};

use crate::core::{
    backends::libp2p::{
        behaviour::{BlendBehaviour, BlendBehaviourEvent},
        Libp2pBlendBackendSettings, LOG_TARGET,
    },
    settings::BlendConfig,
};

#[derive(Debug)]
pub enum BlendSwarmMessage {
    Publish(EncapsulatedMessage),
}

pub(super) struct BlendSwarm<SessionStream, Rng>
where
    Rng: 'static,
{
    swarm: Swarm<BlendBehaviour<Rng>>,
    swarm_messages_receiver: mpsc::Receiver<BlendSwarmMessage>,
    incoming_message_sender: broadcast::Sender<UnwrappedMessage>,
    session_stream: SessionStream,
    latest_session_info: Membership<PeerId>,
    rng: Rng,
    peering_degree: usize,
}

impl<SessionStream, Rng> BlendSwarm<SessionStream, Rng>
where
    Rng: RngCore + Clone,
{
    pub(super) fn new(
        config: BlendConfig<Libp2pBlendBackendSettings, PeerId>,
        session_stream: SessionStream,
        mut rng: Rng,
        swarm_messages_receiver: mpsc::Receiver<BlendSwarmMessage>,
        incoming_message_sender: broadcast::Sender<UnwrappedMessage>,
    ) -> Self {
        let membership = config.membership();
        let keypair = Keypair::from(ed25519::Keypair::from(config.backend.node_key.clone()));
        let mut swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_behaviour(|_| BlendBehaviour::new(&config, rng.clone()))
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

        // Dial the initial peers randomly selected
        membership
            .choose_remote_nodes(&mut rng, config.backend.peering_degree)
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
            peering_degree: config.backend.peering_degree,
        }
    }
}

impl<SessionStream, Rng> BlendSwarm<SessionStream, Rng> {
    fn handle_swarm_message(&mut self, msg: BlendSwarmMessage) {
        match msg {
            BlendSwarmMessage::Publish(msg) => {
                self.handle_publish_swarm_message(msg);
            }
        }
    }

    fn handle_publish_swarm_message(&mut self, msg: EncapsulatedMessage) {
        if let Err(e) = self.swarm.behaviour_mut().blend.validate_and_publish(msg) {
            tracing::error!(target: LOG_TARGET, "Failed to publish message to blend network: {e:?}");
            tracing::info!(counter.failed_outbound_messages = 1);
        } else {
            tracing::info!(counter.successful_outbound_messages = 1);
        }
    }

    fn handle_blend_message(&self, msg: UnwrappedMessage) {
        if let Err(e) = self.incoming_message_sender.send(msg) {
            tracing::error!(target: LOG_TARGET, "Failed to send incoming message to channel: {e}");
            tracing::info!(counter.failed_inbound_messages = 1);
        } else {
            tracing::info!(counter.successful_inbound_messages = 1);
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

    fn handle_blend_behaviour_event(&mut self, blend_event: nomos_blend_network::core::Event) {
        match blend_event {
            nomos_blend_network::core::Event::Message(msg) => {
                self.handle_blend_message(*msg);
            }
            nomos_blend_network::core::Event::SpammyPeer(peer_id) => {
                self.handle_spammy_peer(peer_id);
            }
            nomos_blend_network::core::Event::UnhealthyPeer(peer_id) => {
                self.handle_unhealthy_peer(peer_id);
            }
            nomos_blend_network::core::Event::HealthyPeer(peer_id) => {
                Self::handle_healthy_peer(peer_id);
            }
            nomos_blend_network::core::Event::Error(e) => {
                tracing::error!(target: LOG_TARGET, "Received error from blend network: {e:?}");
                self.check_and_dial_new_peers();
                tracing::info!(counter.error = 1);
            }
        }
    }

    fn handle_event(&mut self, event: SwarmEvent<BlendBehaviourEvent<Rng>>) {
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
                    target: LOG_TARGET,
                    "Connection closed: peer:{}, conn_id:{}",
                    peer_id,
                    connection_id
                );
                self.check_and_dial_new_peers();
            }
            _ => {
                tracing::debug!(target: LOG_TARGET, "Received event from blend network that will be ignored.");
                tracing::info!(counter.ignored_event = 1);
            }
        }
    }

    fn handle_spammy_peer(&mut self, peer_id: PeerId) {
        tracing::debug!(target: LOG_TARGET, "Peer {} is spammy", peer_id);
        self.swarm.behaviour_mut().blocked_peers.block_peer(peer_id);
        self.check_and_dial_new_peers();
    }

    fn handle_unhealthy_peer(&mut self, peer_id: PeerId) {
        tracing::debug!(target: LOG_TARGET, "Peer {} is unhealthy", peer_id);
        self.check_and_dial_new_peers();
    }

    fn handle_healthy_peer(peer_id: PeerId) {
        tracing::debug!(target: LOG_TARGET, "Peer {} is healthy", peer_id);
    }

    /// Dial new peers, if necessary, to maintain the peering degree.
    /// We aim to have at least the peering degree number of "healthy" peers.
    fn check_and_dial_new_peers(&mut self) {
        let num_new_conns_needed = self
            .peering_degree
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
