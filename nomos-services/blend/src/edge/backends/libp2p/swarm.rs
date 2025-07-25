use std::{collections::HashMap, time::Duration};

use futures::{AsyncWriteExt as _, Stream, StreamExt as _};
use libp2p::{
    identity::Keypair,
    swarm::{dial_opts::PeerCondition, ConnectionId},
    PeerId, Swarm, SwarmBuilder,
};
use nomos_blend_network::{send_msg, PROTOCOL_NAME};
use nomos_blend_scheduling::membership::{Membership, Node};
use nomos_libp2p::{ed25519, DialError, DialOpts, SwarmEvent};
use rand::RngCore;
use tokio::sync::mpsc;
use tracing::{debug, error, trace};

use super::settings::Libp2pBlendBackendSettings;
use crate::edge::backends::libp2p::LOG_TARGET;

pub(super) struct BlendSwarm<SessionStream, Rng>
where
    Rng: RngCore + 'static,
{
    swarm: Swarm<libp2p_stream::Behaviour>,
    stream_control: libp2p_stream::Control,
    command_receiver: mpsc::Receiver<Command>,
    session_stream: SessionStream,
    current_membership: Option<Membership<PeerId>>,
    rng: Rng,
    pending_dials: HashMap<(PeerId, ConnectionId), Vec<u8>>,
}

#[derive(Debug)]
pub enum Command {
    SendMessage(Vec<u8>),
}

impl<SessionStream, Rng> BlendSwarm<SessionStream, Rng>
where
    Rng: RngCore + 'static,
{
    pub(super) fn new(
        settings: &Libp2pBlendBackendSettings,
        session_stream: SessionStream,
        current_membership: Option<Membership<PeerId>>,
        rng: Rng,
        command_receiver: mpsc::Receiver<Command>,
    ) -> Self {
        let keypair = Keypair::from(ed25519::Keypair::from(settings.node_key.clone()));
        let swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_behaviour(|_| libp2p_stream::Behaviour::new())
            .expect("Behaviour should be built")
            .with_swarm_config(|cfg| {
                // The idle timeout starts ticking once there are no active streams on a
                // connection. We want the connection to be closed as soon as
                // all streams are dropped.
                cfg.with_idle_connection_timeout(Duration::ZERO)
            })
            .build();
        let stream_control = swarm.behaviour().new_control();
        Self {
            swarm,
            stream_control,
            command_receiver,
            session_stream,
            current_membership,
            rng,
            pending_dials: HashMap::new(),
        }
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::SendMessage(msg) => {
                self.handle_send_message_command(msg);
            }
        }
    }

    fn handle_send_message_command(&mut self, msg: Vec<u8>) {
        self.dial_and_schedule_message(msg);
    }

    fn dial_and_schedule_message(&mut self, msg: Vec<u8>) {
        let Some(peer) = self.choose_peer() else {
            error!(target: LOG_TARGET, "No peers available to send the message to");
            return;
        };

        let opts = DialOpts::peer_id(peer.id)
            .addresses(vec![peer.address.clone()])
            .condition(PeerCondition::Always)
            .build();
        let connection_id = opts.connection_id();
        if let Err(e) = self.swarm.dial(opts) {
            error!(target: LOG_TARGET, "Failed to dial peer {}: {e}", peer.id);
            return;
        }
        debug!(target: LOG_TARGET, "Message scheduled for the dial: peer:{}, connection_id:{}", peer.id, connection_id);
        self.pending_dials.insert((peer.id, connection_id), msg);
    }

    fn choose_peer(&mut self) -> Option<Node<PeerId>> {
        let Some(membership) = &self.current_membership else {
            return None;
        };
        membership
            .choose_remote_nodes(&mut self.rng, 1)
            .next()
            .cloned()
    }

    async fn handle_swarm_event(&mut self, event: SwarmEvent<()>) {
        match event {
            SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                ..
            } => {
                self.handle_connection_established(peer_id, connection_id)
                    .await;
            }
            SwarmEvent::OutgoingConnectionError {
                connection_id,
                peer_id,
                error,
            } => {
                self.handle_outgoing_connection_error(peer_id, connection_id, &error);
            }
            _ => {
                trace!(target: LOG_TARGET, "Unhandled swarm event: {event:?}");
            }
        }
    }

    async fn handle_connection_established(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
    ) {
        debug!(target: LOG_TARGET, "Connection established: peer_id:{peer_id}, connection_id:{connection_id}");

        let Some(message) = self.pending_dials.remove(&(peer_id, connection_id)) else {
            debug!(target: LOG_TARGET, "No message assigned to this connection. Ignoring: peer_id:{peer_id}, connection_id:{connection_id}");
            return;
        };

        match self
            .stream_control
            .open_stream(peer_id, PROTOCOL_NAME)
            .await
        {
            Ok(stream) => Self::send_message_to_stream(message, stream).await,
            Err(e) => {
                error!(target: LOG_TARGET, "Failed to open stream to {peer_id}: {e}");
            }
        }
    }

    async fn send_message_to_stream(message: Vec<u8>, stream: libp2p::Stream) {
        match send_msg(stream, message).await {
            Ok(stream) => {
                debug!(target: LOG_TARGET, "Message sent successfully");
                Self::close_stream(stream).await;
            }
            Err(e) => {
                error!(target: LOG_TARGET, "Failed to send message: {e}");
            }
        }
    }

    async fn close_stream(mut stream: libp2p::Stream) {
        if let Err(e) = stream.close().await {
            error!(target: LOG_TARGET, "Failed to close stream: {e}");
        }
    }

    fn handle_outgoing_connection_error(
        &mut self,
        peer_id: Option<PeerId>,
        connection_id: ConnectionId,
        error: &DialError,
    ) {
        error!(target: LOG_TARGET, "Outgoing connection error: peer_id:{peer_id:?}, connection_id:{connection_id}: {error}");

        let Some(peer_id) = peer_id else {
            debug!(target: LOG_TARGET, "No PeerId set. Ignoring: peer_id:{peer_id:?}, connection_id:{connection_id}");
            return;
        };
        let Some(message) = self.pending_dials.remove(&(peer_id, connection_id)) else {
            debug!(target: LOG_TARGET, "No message assigned to this connection. Ignoring: peer_id:{peer_id}, connection_id:{connection_id}");
            return;
        };

        self.dial_and_schedule_message(message);
    }

    // TODO: Implement the actual session transition.
    //       https://github.com/logos-co/nomos/issues/1462
    fn transition_session(&mut self, membership: Membership<PeerId>) {
        self.current_membership = Some(membership);
    }
}

impl<SessionStream, Rng> BlendSwarm<SessionStream, Rng>
where
    Rng: RngCore + 'static,
    SessionStream: Stream<Item = Membership<PeerId>> + Unpin,
{
    pub(super) async fn run(mut self) {
        loop {
            tokio::select! {
                Some(event) = self.swarm.next() => {
                    self.handle_swarm_event(event).await;
                }
                Some(command) = self.command_receiver.recv() => {
                    self.handle_command(command);
                }
                Some(new_session) = self.session_stream.next() => {
                    self.transition_session(new_session);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ops::RangeInclusive;

    use libp2p::Multiaddr;
    use nomos_blend_message::crypto::Ed25519PrivateKey;
    use nomos_blend_network::core::{self, Event};
    use rand::rngs::OsRng;
    use tokio::time::Instant;

    use super::*;

    /// Tests if a message sent by [`BlendEdgeSwarm`] is received by
    /// [`core::Behaviour`].
    #[test_log::test(tokio::test)]
    async fn send_message() {
        // Initialize a Swarm with [`core::Behaviour`] for a core node.
        let (mut peer_swarm, peer_info) = core_node_swarm().await;

        // Initialize and run a [`BlendEdgeSwarm`].
        let (command_sender, command_receiver) = mpsc::channel(1);
        let swarm = BlendSwarm::new(
            &Libp2pBlendBackendSettings {
                node_key: ed25519::SecretKey::generate(),
            },
            futures::stream::pending(),
            Some(Membership::new(&[peer_info], None)),
            OsRng,
            command_receiver,
        );
        tokio::spawn(async move {
            swarm.run().await;
        });

        // Schedule a message, expecting that a connection is established,
        // and the message is sent.
        command_sender
            .send(Command::SendMessage(b"hello".to_vec()))
            .await
            .expect("Failed to send command");

        // Wait for the message to be received by the core node.
        let mut timeout = Box::pin(tokio::time::sleep_until(
            Instant::now() + Duration::from_secs(3),
        ));
        loop {
            tokio::select! {
                () = &mut timeout => {
                    panic!("Timeout waiting for message");
                }
                event = peer_swarm.next() => {
                    if let Some(SwarmEvent::Behaviour(Event::Message(message))) = event {
                        assert_eq!(message, b"hello");
                        break;
                    }
                }
            }
        }
    }

    /// Creates a Swarm with [`core::Behaviour`].
    async fn core_node_swarm() -> (
        Swarm<core::Behaviour<ObservationWindowClockProvider>>,
        Node<PeerId>,
    ) {
        let key = Keypair::generate_ed25519();
        let mut node = Node {
            id: key.public().to_peer_id(),
            address: Multiaddr::empty(),
            public_key: Ed25519PrivateKey::generate().public_key(),
        };
        let behaviour = core::Behaviour::new(
            &core::Config {
                seen_message_cache_size: 1,
            },
            ObservationWindowClockProvider,
            Some(Membership::new(&[node.clone()], Some(&node.public_key))),
            Duration::from_secs(1),
        );

        let mut swarm = SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_quic()
            .with_behaviour(|_| behaviour)
            .unwrap()
            .build();
        let _ = swarm
            .listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap())
            .unwrap();
        let listening_addr = wait_for_listening_address(&mut swarm).await;
        node.address = listening_addr;
        (swarm, node)
    }

    async fn wait_for_listening_address(
        swarm: &mut Swarm<core::Behaviour<ObservationWindowClockProvider>>,
    ) -> Multiaddr {
        loop {
            if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
                break address;
            }
        }
    }

    struct ObservationWindowClockProvider;

    impl core::IntervalStreamProvider for ObservationWindowClockProvider {
        type IntervalStream = futures::stream::Pending<Self::IntervalItem>;
        type IntervalItem = RangeInclusive<u64>;

        fn interval_stream(&self) -> Self::IntervalStream {
            futures::stream::pending()
        }
    }
}
