use core::num::NonZeroU64;
use std::{
    collections::{hash_map::Entry, HashMap},
    io,
    time::Duration,
};

use futures::{AsyncWriteExt as _, Stream, StreamExt as _};
use libp2p::{
    swarm::{dial_opts::PeerCondition, ConnectionId},
    Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use libp2p_stream::OpenStreamError;
use nomos_blend_network::send_msg;
use nomos_blend_scheduling::{
    membership::{Membership, Node},
    serialize_encapsulated_message, EncapsulatedMessage,
};
use nomos_libp2p::{DialError, DialOpts, SwarmEvent};
use rand::RngCore;
use tokio::sync::mpsc;
use tracing::{debug, error, trace};

use super::settings::Libp2pBlendBackendSettings;
use crate::edge::backends::libp2p::LOG_TARGET;

#[derive(Debug)]
pub struct DialAttempt {
    /// Address of peer being dialed.
    address: Multiaddr,
    /// The latest (ongoing) attempt number.
    attempt_number: NonZeroU64,
    /// The message to send once the peer is successfully dialed.
    message: EncapsulatedMessage,
}

#[cfg(test)]
impl DialAttempt {
    pub const fn address(&self) -> &Multiaddr {
        &self.address
    }

    pub const fn attempt_number(&self) -> NonZeroU64 {
        self.attempt_number
    }

    pub const fn message(&self) -> &EncapsulatedMessage {
        &self.message
    }
}

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
    max_dial_attempts_per_connection: NonZeroU64,
    pending_dials: HashMap<(PeerId, ConnectionId), DialAttempt>,
    protocol_name: StreamProtocol,
}

#[derive(Debug)]
pub enum Command {
    SendMessage(EncapsulatedMessage),
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
        protocol_name: StreamProtocol,
    ) -> Self {
        let keypair = settings.keypair();
        let swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_behaviour(|_| libp2p_stream::Behaviour::new())
            .expect("Behaviour should be built")
            .with_swarm_config(|cfg| {
                // We cannot use zero as that would immediately close a connection with an edge
                // node before they have a chance to upgrade the stream and send the message.
                cfg.with_idle_connection_timeout(Duration::from_secs(1))
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
            max_dial_attempts_per_connection: settings.max_dial_attempts_per_peer_per_message,
            protocol_name,
        }
    }

    #[cfg(test)]
    pub fn new_test(
        membership: Option<Membership<PeerId>>,
        command_receiver: mpsc::Receiver<Command>,
        max_dial_attempts_per_connection: NonZeroU64,
        rng: Rng,
        session_stream: SessionStream,
        protocol_name: StreamProtocol,
    ) -> Self {
        use crate::test_utils::memory_test_swarm;

        let inner_swarm =
            memory_test_swarm(Duration::from_secs(1), |_| libp2p_stream::Behaviour::new());

        Self {
            command_receiver,
            current_membership: membership,
            max_dial_attempts_per_connection,
            pending_dials: HashMap::new(),
            rng,
            session_stream,
            stream_control: inner_swarm.behaviour().new_control(),
            swarm: inner_swarm,
            protocol_name,
        }
    }

    #[cfg(test)]
    pub const fn pending_dials(&self) -> &HashMap<(PeerId, ConnectionId), DialAttempt> {
        &self.pending_dials
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::SendMessage(msg) => {
                self.handle_send_message_command(msg);
            }
        }
    }

    fn handle_send_message_command(&mut self, msg: EncapsulatedMessage) {
        self.dial_and_schedule_message_except(msg, None);
    }

    /// Schedule a dial with retries for a given message.
    ///
    /// The peer to send the message to is chosen at random, except the provided
    /// peer, if specified.
    fn dial_and_schedule_message_except(
        &mut self,
        msg: EncapsulatedMessage,
        except: Option<PeerId>,
    ) {
        let Some(node) = self.choose_peer_except(except) else {
            error!(target: LOG_TARGET, "No peers available to send the message to");
            return;
        };
        let (peer_id, address) = (node.id, node.address);
        let opts = dial_opts(peer_id, address.clone());
        let connection_id = opts.connection_id();

        let Entry::Vacant(empty_entry) = self.pending_dials.entry((peer_id, connection_id)) else {
            panic!("Dial attempt for peer {peer_id:?} and connection {connection_id:?} should not be present in storage.");
        };
        empty_entry.insert(DialAttempt {
            address,
            attempt_number: 1.try_into().unwrap(),
            message: msg,
        });

        if let Err(e) = self.swarm.dial(opts) {
            error!(target: LOG_TARGET, "Failed to dial peer {peer_id:?} on connection {connection_id:?}: {e:?}");
            self.retry_dial(peer_id, connection_id);
        }
    }

    /// Attempt to retry dialing the specified peer, if the maximum attempts
    /// have not already been performed.
    ///
    /// It returns `None` if a new dial attempt is performed, `Some` otherwise
    /// with the dial details of the peer that has been removed from the map
    /// of ongoing dials.
    fn retry_dial(&mut self, peer_id: PeerId, connection_id: ConnectionId) -> Option<DialAttempt> {
        let dial_attempt = self
            .pending_dials
            .remove(&(peer_id, connection_id))
            .unwrap();
        let new_dial_attempt_number = dial_attempt.attempt_number.checked_add(1).unwrap();
        if new_dial_attempt_number > self.max_dial_attempts_per_connection {
            return Some(dial_attempt);
        }
        let new_dial_opts = dial_opts(peer_id, dial_attempt.address.clone());
        self.pending_dials.insert(
            (peer_id, new_dial_opts.connection_id()),
            DialAttempt {
                attempt_number: new_dial_attempt_number,
                ..dial_attempt
            },
        );

        if let Err(e) = self.swarm.dial(new_dial_opts) {
            error!(target: LOG_TARGET, "Failed to redial peer {peer_id:?}: {e:?}");
            self.retry_dial(peer_id, connection_id);
        }
        None
    }

    fn choose_peer_except(&mut self, except: Option<PeerId>) -> Option<Node<PeerId>> {
        let Some(membership) = &self.current_membership else {
            return None;
        };
        membership
            .filter_and_choose_remote_nodes(&mut self.rng, 1, &except.into_iter().collect())
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
        debug!(target: LOG_TARGET, "Connection established: peer_id: {peer_id}, connection_id: {connection_id}");

        // We need to clone so we can access `&mut self` below.
        let message = self
            .pending_dials
            .get(&(peer_id, connection_id))
            .map(|entry| entry.message.clone())
            .unwrap();

        match self
            .stream_control
            .open_stream(peer_id, self.protocol_name.clone())
            .await
        {
            Ok(stream) => {
                self.handle_open_stream_success(stream, &message, (peer_id, connection_id))
                    .await;
            }
            Err(e) => self.handle_open_stream_failure(&e, (peer_id, connection_id)),
        }
    }

    async fn handle_open_stream_success(
        &mut self,
        stream: libp2p::Stream,
        message: &EncapsulatedMessage,
        (peer_id, connection_id): (PeerId, ConnectionId),
    ) {
        match send_msg(stream, serialize_encapsulated_message(message)).await {
            Ok(stream) => {
                self.handle_send_message_success(stream, (peer_id, connection_id))
                    .await;
            }
            Err(e) => self.handle_send_message_failure(&e, (peer_id, connection_id)),
        }
    }

    async fn handle_send_message_success(
        &mut self,
        stream: libp2p::Stream,
        (peer_id, connection_id): (PeerId, ConnectionId),
    ) {
        debug!(target: LOG_TARGET, "Message sent successfully to peer {peer_id:?} on connection {connection_id:?}.");
        close_stream(stream, peer_id, connection_id).await;
        // Regardless of the result of closing the stream, the message was sent so we
        // can remove the pending dial info.
        self.pending_dials.remove(&(peer_id, connection_id));
    }

    fn handle_send_message_failure(
        &mut self,
        error: &io::Error,
        (peer_id, connection_id): (PeerId, ConnectionId),
    ) {
        error!(target: LOG_TARGET, "Failed to send message: {error} to peer {peer_id:?} on connection {connection_id:?}.");
        // If the maximum attempt count was reached for this peer, try to schedule the
        // message for a different peer.
        if let Some(DialAttempt { message, .. }) = self.retry_dial(peer_id, connection_id) {
            self.dial_and_schedule_message_except(message, Some(peer_id));
        }
    }

    fn handle_open_stream_failure(
        &mut self,
        error: &OpenStreamError,
        (peer_id, connection_id): (PeerId, ConnectionId),
    ) {
        error!(target: LOG_TARGET, "Failed to open stream to {peer_id}: {error}");
        // If the maximum attempt count was reached for this peer, try to schedule the
        // message for a different peer.
        if let Some(DialAttempt { message, .. }) = self.retry_dial(peer_id, connection_id) {
            self.dial_and_schedule_message_except(message, Some(peer_id));
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

        // If the maximum attempt count was reached for this peer, try to schedule the
        // message for a different peer.
        if let Some(DialAttempt { message, .. }) = self.retry_dial(peer_id, connection_id) {
            self.dial_and_schedule_message_except(message, Some(peer_id));
        }
    }

    // TODO: Implement the actual session transition.
    //       https://github.com/logos-co/nomos/issues/1462
    fn transition_session(&mut self, membership: Membership<PeerId>) {
        self.current_membership = Some(membership);
    }

    #[cfg(test)]
    pub fn send_message(&mut self, msg: EncapsulatedMessage) {
        self.dial_and_schedule_message_except(msg, None);
    }

    #[cfg(test)]
    pub fn send_message_to_anyone_except(&mut self, peer_id: PeerId, msg: EncapsulatedMessage) {
        self.dial_and_schedule_message_except(msg, Some(peer_id));
    }
}

async fn close_stream(mut stream: libp2p::Stream, peer_id: PeerId, connection_id: ConnectionId) {
    if let Err(e) = stream.close().await {
        error!(target: LOG_TARGET, "Failed to close stream: {e} with peer {peer_id:?} on connection {connection_id:?}.");
    }
}

fn dial_opts(peer_id: PeerId, address: Multiaddr) -> DialOpts {
    DialOpts::peer_id(peer_id)
        .addresses(vec![address])
        .condition(PeerCondition::Always)
        .build()
}

impl<SessionStream, Rng> BlendSwarm<SessionStream, Rng>
where
    Rng: RngCore + 'static,
    SessionStream: Stream<Item = Membership<PeerId>> + Unpin,
{
    pub(super) async fn run(mut self) {
        loop {
            self.poll_next_internal().await;
        }
    }

    async fn poll_next_internal(&mut self) {
        self.poll_next_and_match(|_| false).await;
    }

    async fn poll_next_and_match<Predicate>(&mut self, predicate: Predicate) -> bool
    where
        Predicate: Fn(&SwarmEvent<()>) -> bool,
    {
        tokio::select! {
            Some(event) = self.swarm.next() => {
                let predicate_matched = predicate(&event);
                self.handle_swarm_event(event).await;
                predicate_matched
            }
            Some(command) = self.command_receiver.recv() => {
                self.handle_command(command);
                false
            }
            Some(new_session) = self.session_stream.next() => {
                self.transition_session(new_session);
                false
            }
        }
    }

    #[cfg(test)]
    pub async fn poll_next_until<Predicate>(&mut self, predicate: Predicate)
    where
        Predicate: Fn(&SwarmEvent<()>) -> bool + Copy,
    {
        loop {
            if self.poll_next_and_match(predicate).await {
                break;
            }
        }
    }
}
