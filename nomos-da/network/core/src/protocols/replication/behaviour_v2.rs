use std::{
    collections::{hash_map::Entry, HashMap, HashSet, VecDeque},
    task::{Context, Poll, Waker},
};

use either::Either;
use futures::{future::BoxFuture, stream::FuturesUnordered, AsyncWriteExt, FutureExt, StreamExt};
use indexmap::{IndexMap, IndexSet};
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        ConnectionClosed, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler,
        THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, Stream,
};
use libp2p_stream::{Control, IncomingStreams, OpenStreamError};
use log::{error, trace};
use nomos_da_messages::packing::{pack_to_writer, unpack_from_reader};
use subnetworks_assignations::MembershipHandler;
use thiserror::Error;

use crate::{protocol::REPLICATION_PROTOCOL, SubnetworkId};

pub type DaMessage = nomos_da_messages::replication::ReplicationRequest;

#[derive(Debug, Error)]
pub enum ReplicationError {
    #[error("Stream disconnected: {error}")]
    Io {
        peer_id: PeerId,
        error: std::io::Error,
    },
    #[error("Error opening stream [{peer_id}]: {error}")]
    OpenStream {
        peer_id: PeerId,
        error: OpenStreamError,
    },
}

impl ReplicationError {
    pub const fn peer_id(&self) -> Option<&PeerId> {
        match self {
            Self::Io { peer_id, .. } => Some(peer_id),
            Self::OpenStream { peer_id, .. } => Some(peer_id),
        }
    }
}

impl Clone for ReplicationError {
    fn clone(&self) -> Self {
        match self {
            Self::Io { peer_id, error } => Self::Io {
                peer_id: *peer_id,
                error: std::io::Error::new(error.kind(), error.to_string()),
            },
            Self::OpenStream { peer_id, error } => Self::OpenStream {
                peer_id: *peer_id,
                error: match error {
                    OpenStreamError::UnsupportedProtocol(protocol) => {
                        OpenStreamError::UnsupportedProtocol(protocol.clone())
                    }
                    OpenStreamError::Io(error) => {
                        OpenStreamError::Io(std::io::Error::new(error.kind(), error.to_string()))
                    }
                    err => OpenStreamError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        err.to_string(),
                    )),
                },
            },
        }
    }
}

/// Nomos DA BroadcastEvents to be bubble up to logic layers
#[derive(Debug)]
pub enum ReplicationEvent {
    IncomingMessage {
        peer_id: PeerId,
        message: Box<DaMessage>,
    },
    ReplicationError {
        error: ReplicationError,
    },
}

impl From<ReplicationError> for ReplicationEvent {
    fn from(error: ReplicationError) -> Self {
        Self::ReplicationError { error }
    }
}

impl ReplicationEvent {
    pub fn blob_size(&self) -> Option<usize> {
        match self {
            Self::IncomingMessage { message, .. } => Some(message.blob.data.column_len()),
            _ => None,
        }
    }
}

type IncomingTask = BoxFuture<'static, Result<(PeerId, DaMessage, Stream), ReplicationError>>;
type OutgoingTask = BoxFuture<'static, Result<(PeerId, Stream), ReplicationError>>;

enum StreamState {
    Idle(Stream),
    Busy,
}

impl StreamState {
    pub fn take(&mut self) -> Self {
        let mut ret = Self::Busy;
        std::mem::swap(self, &mut ret);
        ret
    }
}

/// Nomos DA broadcast network behaviour
/// This item handles the logic of the nomos da subnetworks broadcasting
/// DA subnetworks are a logical distribution of subsets.
/// A node just connects and accepts connections to other nodes that are in the
/// same subsets. A node forwards messages to all connected peers which are
/// member of the addressed `SubnetworkId`.
pub struct ReplicationBehaviour<Membership> {
    /// Local peer Id, related to the libp2p public key
    local_peer_id: PeerId,
    /// Underlying stream behaviour
    stream_behaviour: libp2p_stream::Behaviour,
    /// Used to open new outgoing streams from the stream behaviour
    control: Control,
    /// Provides inbound streams that are accepted by the stream behaviour
    incoming_streams: IncomingStreams,
    /// Holds tasks for reading messages from incoming streams
    incoming_tasks: FuturesUnordered<IncomingTask>,
    /// Membership handler, membership handles the subsets logics on who is
    /// where in the nomos DA subnetworks
    membership: Membership,
    /// Relation of connected peers of replication subnetworks
    connected: HashSet<PeerId>,
    /// Pending outgoing messages are stored here, they are then consumed by
    /// each respective outgoing stream
    pending_outgoing_messages: IndexMap<PeerId, VecDeque<DaMessage>>,
    /// The last peer whose pending message was scheduled for sending
    previous_scheduled_pending: Option<PeerId>,
    /// Indicates which outgoing streams which are currently idle or busy
    outgoing_streams: HashMap<PeerId, StreamState>,
    /// TODO
    outgoing_tasks: FuturesUnordered<OutgoingTask>,
    /// Seen messages cache holds a record of seen messages, messages will be
    /// removed from this set after some time to keep it
    seen_message_cache: IndexSet<(Vec<u8>, SubnetworkId)>,
    /// Waker that handles polling
    waker: Option<Waker>,
}

impl<Membership> ReplicationBehaviour<Membership> {
    pub fn new(peer_id: PeerId, membership: Membership) -> Self {
        let stream_behaviour = libp2p_stream::Behaviour::new();
        let mut control = stream_behaviour.new_control();
        let incoming_streams = control
            .accept(REPLICATION_PROTOCOL)
            .expect("A unique protocol can be accepted only once");
        let incoming_tasks = FuturesUnordered::new();
        let outgoing_tasks = FuturesUnordered::new();

        Self {
            local_peer_id: peer_id,
            stream_behaviour,
            control,
            incoming_streams,
            incoming_tasks,
            membership,
            connected: Default::default(),
            pending_outgoing_messages: Default::default(),
            previous_scheduled_pending: None,
            outgoing_streams: Default::default(),
            outgoing_tasks,
            seen_message_cache: Default::default(),
            waker: None,
        }
    }

    pub fn update_membership(&mut self, membership: Membership) {
        self.membership = membership;
    }
}

impl<M> ReplicationBehaviour<M>
where
    M: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>,
{
    /// Check if some peer membership lies in at least a single subnetwork that
    /// the local peer is a member too.
    fn is_neighbour(&self, peer_id: &PeerId) -> bool {
        self.membership
            .membership(&self.local_peer_id)
            .intersection(&self.membership.membership(peer_id))
            .count()
            > 0
    }

    fn no_loopback_member_peers_of(&self, subnetwork: &SubnetworkId) -> HashSet<PeerId> {
        let mut peers = self.membership.members_of(subnetwork);
        // no loopback
        peers.remove(&self.local_peer_id);
        peers
    }

    fn replicate_message(&mut self, message: DaMessage) {
        let message_id = (message.blob.blob_id.to_vec(), message.subnetwork_id);
        if self.seen_message_cache.contains(&message_id) {
            return;
        }
        self.seen_message_cache.insert(message_id);
        self.send_message(message)
    }

    pub fn send_message(&mut self, message: DaMessage) {
        // Push a message in the queue for every single peer connected that is a member
        // of the selected subnetwork_id
        let peers = self.no_loopback_member_peers_of(&message.subnetwork_id);

        self.connected
            .iter()
            .filter(|peer_id| peers.contains(peer_id))
            .for_each(|peer_id| {
                self.pending_outgoing_messages
                    .entry(*peer_id)
                    .or_default()
                    .push_back(message.clone());
            });

        self.try_wake();
    }

    pub fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    /// Try to read a single message from the incoming stream
    async fn try_read_message(
        peer_id: PeerId,
        mut stream: Stream,
    ) -> Result<(PeerId, DaMessage, Stream), ReplicationError> {
        let message = unpack_from_reader(&mut stream)
            .await
            .map_err(|error| ReplicationError::Io { peer_id, error })?;
        Ok((peer_id, message, stream))
    }

    /// Open a new stream from the underlying control to the provided peer
    async fn try_open_stream(
        peer_id: PeerId,
        mut control: Control,
    ) -> Result<Stream, ReplicationError> {
        let stream = control
            .open_stream(peer_id, REPLICATION_PROTOCOL)
            .await
            .map_err(|error| ReplicationError::OpenStream { peer_id, error })?;
        Ok(stream)
    }

    /// TODO
    async fn try_write_message(
        peer_id: PeerId,
        message: DaMessage,
        mut stream: Stream,
    ) -> Result<(PeerId, Stream), ReplicationError> {
        pack_to_writer(&message, &mut stream)
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "Message should always be serializable.\nMessage: '{:?}'",
                    message
                )
            });
        stream
            .flush()
            .await
            .map_err(|error| ReplicationError::Io { peer_id, error })?;
        Ok((peer_id, stream))
    }
}

impl<M> NetworkBehaviour for ReplicationBehaviour<M>
where
    M: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId> + 'static,
{
    type ConnectionHandler = Either<
        <libp2p_stream::Behaviour as NetworkBehaviour>::ConnectionHandler,
        libp2p::swarm::dummy::ConnectionHandler,
    >;
    type ToSwarm = ReplicationEvent;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        if !self.is_neighbour(&peer_id) {
            trace!("refusing connection to {peer_id}");
            return Ok(Either::Right(libp2p::swarm::dummy::ConnectionHandler));
        }
        trace!("{}, Connected to {peer_id}", self.local_peer_id);
        self.connected.insert(peer_id);
        self.stream_behaviour
            .handle_established_inbound_connection(connection_id, peer_id, local_addr, remote_addr)
            .map(Either::Left)
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        addr: &Multiaddr,
        role_override: Endpoint,
        port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        trace!("{}, Connected to {peer_id}", self.local_peer_id);
        self.connected.insert(peer_id);
        self.stream_behaviour
            .handle_established_outbound_connection(
                connection_id,
                peer_id,
                addr,
                role_override,
                port_use,
            )
            .map(Either::Left)
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        if let FromSwarm::ConnectionClosed(ConnectionClosed { peer_id, .. }) = event {
            self.connected.remove(&peer_id);
            self.outgoing_streams.remove(&peer_id);
            self.previous_scheduled_pending.map(|last| {
                if last == peer_id {
                    let mut i = self
                        .pending_outgoing_messages
                        .get_index_of(&peer_id)
                        .expect("Peer to be present");
                    // Move the marker back one step, so that the search for the next peer in poll()
                    // continues where we left off, wrap around if necessary
                    if i == 0 {
                        i = self.pending_outgoing_messages.len() - 1;
                    } else {
                        i -= 1;
                    }
                    self.previous_scheduled_pending = self
                        .pending_outgoing_messages
                        .get_index(i)
                        .map(|(id, _)| *id);
                }
            });
            self.pending_outgoing_messages.shift_remove(&peer_id);
        }
        self.stream_behaviour.on_swarm_event(event)
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        let Either::Left(event) = event;
        self.stream_behaviour
            .on_connection_handler_event(peer_id, connection_id, event)
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        let mut should_wake = false;
        // The incoming message to be returned to the swarm **after** all the polling is
        // done, this way we don't starve the tasks that are polled later in the
        // sequence
        let mut incoming_message = None;

        // Check if we've received any new messages
        match self.incoming_tasks.poll_next_unpin(cx) {
            Poll::Ready(Some(Ok((peer_id, message, stream)))) => {
                // Replicate the message to all connected peers from the same subnet if we
                // haven't seen it yet
                self.replicate_message(message.clone());
                // Wait for any next incoming message on the same stream
                self.incoming_tasks
                    .push(Self::try_read_message(peer_id, stream).boxed());
                // Signal to the swarm that we've received the message but poll the other tasks
                // as well first
                incoming_message = Some(Poll::Ready(ToSwarm::GenerateEvent(
                    ReplicationEvent::IncomingMessage {
                        peer_id,
                        message: Box::new(message),
                    },
                )));
            }
            Poll::Ready(Some(Err(error))) => {
                return Poll::Ready(ToSwarm::GenerateEvent(ReplicationEvent::ReplicationError {
                    error,
                }));
            }
            _ => {}
        }
        // If any of the busy outgoing streams has finished sending a message,
        // we can write the next pending message to this stream if there is one
        match self.outgoing_tasks.poll_next_unpin(cx) {
            Poll::Ready(Some(Ok((peer_id, stream)))) => {
                match self
                    .pending_outgoing_messages
                    .get_mut(&peer_id)
                    .expect("At least one message for this peer has been sent")
                    .pop_front()
                {
                    Some(message) => {
                        self.outgoing_tasks
                            .push(Box::pin(Self::try_write_message(peer_id, message, stream)));

                        should_wake = true;
                    }
                    None => {
                        self.outgoing_streams
                            .insert(peer_id, StreamState::Idle(stream));
                    }
                }
            }
            Poll::Ready(Some(Err(error))) => {
                return Poll::Ready(ToSwarm::GenerateEvent(ReplicationEvent::ReplicationError {
                    error,
                }));
            }
            _ => {}
        }

        /// Find the next peer after the previous one that has at least one
        /// pending message and its associated stream is idle or hasn't been
        /// opened yet. Iterate over all peers, starting with the one
        /// after the previous peer, wrap around and finish with the
        /// previous peer at the end. This ensures we're fair scheduling one
        /// message per peer at a time, iterating the peers in a round-robin
        /// fashion.
        fn next_pending(
            previous: &Option<PeerId>,
            pending_outgoing_messages: &IndexMap<PeerId, VecDeque<DaMessage>>,
            mut is_stream_idle_fn: impl FnMut(&PeerId) -> bool,
        ) -> Option<PeerId> {
            match previous {
                Some(previous) => {
                    let i = pending_outgoing_messages
                        .get_index_of(previous)
                        .expect("Peer to be present");
                    pending_outgoing_messages[i + 1..]
                        .iter()
                        .chain(pending_outgoing_messages[..=i].iter())
                        .find_map(|(id, messages)| {
                            (!messages.is_empty() && is_stream_idle_fn(id)).then_some(*id)
                        })
                }
                None => pending_outgoing_messages.iter().find_map(|(id, messages)| {
                    (!messages.is_empty() && is_stream_idle_fn(id)).then_some(*id)
                }),
            }
        }

        // Pick the next peer that has a pending message and an idle or unopened stream
        // and schedule writing the message to it.
        let next = next_pending(
            &self.previous_scheduled_pending,
            &self.pending_outgoing_messages,
            |id| {
                matches!(
                    self.outgoing_streams.get(id),
                    Some(StreamState::Idle(_)) | None
                )
            },
        );

        if let Some(peer_id) = next {
            let message = self
                .pending_outgoing_messages
                .get_mut(&peer_id)
                .expect("Peer is in the map")
                .pop_front()
                .expect("Message is in the queue");
            match self.outgoing_streams.entry(peer_id) {
                Entry::Occupied(mut entry) => match entry.get_mut().take() {
                    StreamState::Idle(stream) => {
                        self.outgoing_tasks
                            .push(Box::pin(Self::try_write_message(peer_id, message, stream)));
                    }
                    StreamState::Busy => {
                        unreachable!("The stream is idle, ensured by next_pending()")
                    }
                },
                // If there is no stream for this peer yet, try to open one, and then write the
                // first message into it
                Entry::Vacant(entry) => {
                    entry.insert(StreamState::Busy);
                    let control = self.control.clone();
                    self.outgoing_tasks.push(Box::pin(async move {
                        let stream = Self::try_open_stream(peer_id, control).await?;
                        let (peer_id, stream) =
                            Self::try_write_message(peer_id, message, stream).await?;
                        Ok((peer_id, stream))
                    }));
                }
            }

            should_wake = true;
        }

        self.previous_scheduled_pending = next;

        // Schedule reading from any new incoming streams if possible
        if let Poll::Ready(Some((peer_id, stream))) = self.incoming_streams.poll_next_unpin(cx) {
            self.incoming_tasks
                .push(Self::try_read_message(peer_id, stream).boxed());
            should_wake = true;
        }

        if let Some(incoming_message) = incoming_message {
            return incoming_message;
        }

        // Always use the waker from the most recent context
        self.waker = Some(cx.waker().clone());

        // Only wake if we have a reason to
        if should_wake {
            self.try_wake();
        }

        Poll::Pending
    }
}
