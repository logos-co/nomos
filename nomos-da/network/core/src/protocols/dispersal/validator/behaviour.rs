use std::task::{Context, Poll};

use either::Either;
use futures::{
    future::BoxFuture, stream::FuturesUnordered, AsyncWriteExt as _, FutureExt as _, StreamExt as _,
};
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent,
        THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, Stream,
};
use libp2p_stream::IncomingStreams;
use log::debug;
use nomos_da_messages::{
    dispersal,
    packing::{pack_to_writer, unpack_from_reader},
};
use subnetworks_assignations::MembershipHandler;
use thiserror::Error;

use crate::{protocol::DISPERSAL_PROTOCOL, SubnetworkId};

#[derive(Debug, Error)]
pub enum DispersalError {
    #[error("Stream disconnected: {error}")]
    Io {
        peer_id: PeerId,
        error: std::io::Error,
    },
}

impl DispersalError {
    #[must_use]
    pub const fn peer_id(&self) -> Option<&PeerId> {
        match self {
            Self::Io { peer_id, .. } => Some(peer_id),
        }
    }
}

impl Clone for DispersalError {
    fn clone(&self) -> Self {
        match self {
            Self::Io { peer_id, error } => Self::Io {
                peer_id: *peer_id,
                error: std::io::Error::new(error.kind(), error.to_string()),
            },
        }
    }
}

#[derive(Debug)]
pub enum DispersalEvent {
    /// Received a n
    IncomingMessage {
        message: Box<dispersal::DispersalRequest>,
    },
    /// Something went wrong receiving the blob
    DispersalError { error: DispersalError },
}

impl DispersalEvent {
    #[must_use]
    pub fn share_size(&self) -> Option<usize> {
        match self {
            Self::IncomingMessage { message } => Some(message.share.data.column_len()),
            Self::DispersalError { .. } => None,
        }
    }
}

type DispersalTask =
    BoxFuture<'static, Result<(PeerId, dispersal::DispersalRequest, Stream), DispersalError>>;

pub struct DispersalValidatorBehaviour<Membership> {
    stream_behaviour: libp2p_stream::Behaviour,
    incoming_streams: IncomingStreams,
    tasks: FuturesUnordered<DispersalTask>,
    membership: Membership,
}

impl<Membership: MembershipHandler> DispersalValidatorBehaviour<Membership> {
    pub fn new(membership: Membership) -> Self {
        let stream_behaviour = libp2p_stream::Behaviour::new();
        let mut stream_control = stream_behaviour.new_control();
        let incoming_streams = stream_control
            .accept(DISPERSAL_PROTOCOL)
            .expect("Just a single accept to protocol is valid");
        let tasks = FuturesUnordered::new();
        Self {
            stream_behaviour,
            incoming_streams,
            tasks,
            membership,
        }
    }

    pub fn update_membership(&mut self, membership: Membership) {
        self.membership = membership;
    }

    /// Stream handling messages task.
    /// This task handles a single message receive. Then it writes up the
    /// acknowledgment into the same stream as response and finish.
    async fn handle_new_stream(
        peer_id: PeerId,
        mut stream: Stream,
    ) -> Result<(PeerId, dispersal::DispersalRequest, Stream), DispersalError> {
        let message: dispersal::DispersalRequest = unpack_from_reader(&mut stream)
            .await
            .map_err(|error| DispersalError::Io { peer_id, error })?;

        let blob_id = message.share.blob_id;
        let response = dispersal::DispersalResponse::BlobId(blob_id);

        pack_to_writer(&response, &mut stream)
            .await
            .map_err(|error| DispersalError::Io { peer_id, error })?;

        stream
            .flush()
            .await
            .map_err(|error| DispersalError::Io { peer_id, error })?;

        Ok((peer_id, message, stream))
    }
}

impl<M: MembershipHandler<Id = PeerId, NetworkId = SubnetworkId> + 'static> NetworkBehaviour
    for DispersalValidatorBehaviour<M>
{
    type ConnectionHandler = Either<
        <libp2p_stream::Behaviour as NetworkBehaviour>::ConnectionHandler,
        libp2p::swarm::dummy::ConnectionHandler,
    >;
    type ToSwarm = DispersalEvent;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        if !self.membership.is_allowed(&peer) {
            return Ok(Either::Right(libp2p::swarm::dummy::ConnectionHandler));
        }
        self.stream_behaviour
            .handle_established_inbound_connection(connection_id, peer, local_addr, remote_addr)
            .map(Either::Left)
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(Either::Right(libp2p::swarm::dummy::ConnectionHandler))
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        self.stream_behaviour.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        let Either::Left(event) = event;
        self.stream_behaviour
            .on_connection_handler_event(peer_id, connection_id, event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        let Self {
            incoming_streams,
            tasks,
            ..
        } = self;
        match tasks.poll_next_unpin(cx) {
            Poll::Ready(Some(Ok((peer_id, message, stream)))) => {
                tasks.push(Self::handle_new_stream(peer_id, stream).boxed());
                cx.waker().wake_by_ref();
                return Poll::Ready(ToSwarm::GenerateEvent(DispersalEvent::IncomingMessage {
                    message: Box::new(message),
                }));
            }
            Poll::Ready(Some(Err(error))) => {
                debug!("Error on dispersal stream {error:?}");
            }
            _ => {}
        }
        if let Poll::Ready(Some((peer_id, stream))) = incoming_streams.poll_next_unpin(cx) {
            tasks.push(Self::handle_new_stream(peer_id, stream).boxed());
            cx.waker().wake_by_ref();
        }

        Poll::Pending
    }
}
