use std::{
    collections::{HashMap, VecDeque},
    task::{Context, Poll},
};

use either::Either;
use futures::{
    channel::{
        oneshot,
        oneshot::{Canceled, Receiver, Sender},
    },
    future::BoxFuture,
    stream::{BoxStream, FuturesUnordered},
    AsyncWriteExt, FutureExt, StreamExt,
};
use kzgrs_backend::common::{blob::DaLightBlob, ColumnIndex};
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dial_opts::DialOpts, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler,
        THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, Stream,
};
use libp2p_stream::{Control, IncomingStreams, OpenStreamError};
use nomos_core::{da::BlobId, wire};
use nomos_da_messages::{
    common,
    packing::{pack_to_writer, unpack_from_reader},
    sampling,
};
use subnetworks_assignations::MembershipHandler;
use thiserror::Error;
use tokio::sync::{mpsc, mpsc::UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::error;

use crate::{protocol::SAMPLING_PROTOCOL, SubnetworkId};

#[derive(Debug, Error)]
pub enum SamplingError {
    #[error("Stream disconnected: {error}")]
    Io {
        peer_id: PeerId,
        error: std::io::Error,
    },
    #[error("Dispersal response error: {error:?}")]
    Protocol {
        subnetwork_id: SubnetworkId,
        peer_id: PeerId,
        error: sampling::SampleError,
    },
    #[error("Error opening stream [{peer_id}]: {error}")]
    OpenStream {
        peer_id: PeerId,
        error: OpenStreamError,
    },
    #[error("Unable to deserialize blob response: {error}")]
    Deserialize {
        blob_id: BlobId,
        subnetwork_id: SubnetworkId,
        peer_id: PeerId,
        error: wire::Error,
    },
    #[error("Error sending request: {request:?}")]
    RequestChannel {
        request: BehaviourSampleReq,
        peer_id: PeerId,
    },
    #[error("Malformed blob id: {blob_id:?}")]
    InvalidBlobId { peer_id: PeerId, blob_id: Vec<u8> },
    #[error("Blob not found: {blob_id:?}")]
    BlobNotFound {
        peer_id: PeerId,
        blob_id: Vec<u8>,
        subnetwork_id: SubnetworkId,
    },
    #[error("Canceled response: {error}")]
    ResponseChannel { error: Canceled, peer_id: PeerId },
}

impl SamplingError {
    #[must_use]
    pub const fn peer_id(&self) -> Option<&PeerId> {
        match self {
            Self::Io { peer_id, .. }
            | Self::Protocol { peer_id, .. }
            | Self::OpenStream { peer_id, .. }
            | Self::Deserialize { peer_id, .. }
            | Self::RequestChannel { peer_id, .. }
            | Self::ResponseChannel { peer_id, .. }
            | Self::InvalidBlobId { peer_id, .. }
            | Self::BlobNotFound { peer_id, .. } => Some(peer_id),
        }
    }

    #[must_use]
    pub fn blob_id(&self) -> Option<&BlobId> {
        match self {
            Self::BlobNotFound { blob_id, .. } => blob_id.as_slice().try_into().ok(),
            Self::Deserialize { blob_id, .. } => Some(blob_id),
            Self::Protocol { error, .. } => Some(&error.blob_id),
            _ => None,
        }
    }
}

impl Clone for SamplingError {
    fn clone(&self) -> Self {
        match self {
            Self::Io { peer_id, error } => Self::Io {
                peer_id: *peer_id,
                error: std::io::Error::new(error.kind(), error.to_string()),
            },
            Self::Protocol {
                subnetwork_id,
                peer_id,
                error,
            } => Self::Protocol {
                subnetwork_id: *subnetwork_id,
                peer_id: *peer_id,
                error: error.clone(),
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
                    err => OpenStreamError::Io(std::io::Error::other(err.to_string())),
                },
            },
            Self::Deserialize {
                blob_id,
                subnetwork_id,
                peer_id,
                error,
            } => Self::Deserialize {
                blob_id: *blob_id,
                subnetwork_id: *subnetwork_id,
                peer_id: *peer_id,
                error: error.clone(),
            },
            Self::RequestChannel { request, peer_id } => Self::RequestChannel {
                request: request.clone(),
                peer_id: *peer_id,
            },
            Self::ResponseChannel { error, peer_id } => Self::ResponseChannel {
                peer_id: *peer_id,
                error: *error,
            },
            Self::InvalidBlobId { blob_id, peer_id } => Self::InvalidBlobId {
                peer_id: *peer_id,
                blob_id: blob_id.clone(),
            },
            Self::BlobNotFound {
                blob_id,
                peer_id,
                subnetwork_id,
            } => Self::BlobNotFound {
                peer_id: *peer_id,
                blob_id: blob_id.clone(),
                subnetwork_id: *subnetwork_id,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct BehaviourSampleReq {
    pub blob_id: BlobId,
    pub column_idx: ColumnIndex,
}

impl TryFrom<sampling::SampleRequest> for BehaviourSampleReq {
    type Error = Vec<u8>;

    fn try_from(req: sampling::SampleRequest) -> Result<Self, Self::Error> {
        let sampling::SampleRequest {
            blob_id,
            column_idx,
        } = req;

        Ok(Self {
            blob_id,
            column_idx,
        })
    }
}

#[derive(Debug)]
pub enum BehaviourSampleRes {
    SamplingSuccess {
        blob_id: BlobId,
        subnetwork_id: SubnetworkId,
        blob: Box<DaLightBlob>,
    },
    SampleNotFound {
        blob_id: BlobId,
        subnetwork_id: SubnetworkId,
    },
}

impl From<BehaviourSampleRes> for sampling::SampleResponse {
    fn from(res: BehaviourSampleRes) -> Self {
        match res {
            BehaviourSampleRes::SamplingSuccess { blob, blob_id, .. } => {
                Self::Blob(common::LightBlob::new(blob_id, *blob))
            }
            BehaviourSampleRes::SampleNotFound {
                blob_id,
                subnetwork_id,
            } => Self::Error(sampling::SampleError::new(
                blob_id,
                subnetwork_id,
                sampling::SampleErrorType::NotFound,
                "Sample not found",
            )),
        }
    }
}

#[derive(Debug)]
pub enum SamplingEvent {
    /// A blob successfully arrived its destination
    SamplingSuccess {
        blob_id: BlobId,
        subnetwork_id: SubnetworkId,
        light_blob: Box<DaLightBlob>,
    },
    IncomingSample {
        request_receiver: Receiver<BehaviourSampleReq>,
        response_sender: Sender<BehaviourSampleRes>,
    },
    SamplingError {
        error: SamplingError,
    },
}

/// Auxiliary struct that binds a stream with the corresponding `PeerId`
struct SampleStream {
    stream: Stream,
    peer_id: PeerId,
}

/// Auxiliary struct that binds where to send a request and the pair channel to
/// listen for a response
struct ResponseChannel {
    request_sender: Sender<BehaviourSampleReq>,
    response_receiver: Receiver<BehaviourSampleRes>,
}

type StreamWriterFutureSuccess = (BlobId, SubnetworkId, sampling::SampleResponse, SampleStream);
type StreamWriterFutureError = (SamplingError, Option<SampleStream>);
type OutgoingStreamHandlerFuture =
    BoxFuture<'static, Result<StreamWriterFutureSuccess, StreamWriterFutureError>>;
type IncomingStreamHandlerFuture =
    BoxFuture<'static, Result<SampleStream, (SamplingError, SampleStream)>>;
/// Executor sampling protocol
/// Takes care of sending and replying sampling requests
pub struct SamplingBehaviour<Membership: MembershipHandler> {
    /// Self peer id
    peer_id: PeerId,

    /// Underlying stream behaviour
    stream_behaviour: libp2p_stream::Behaviour,
    /// Incoming sample request streams
    incoming_streams: IncomingStreams,
    /// Underlying stream control
    control: Control,
    /// Pending outgoing running tasks (one task per stream)
    outgoing_tasks: FuturesUnordered<OutgoingStreamHandlerFuture>,
    /// Pending incoming running tasks (one task per stream)
    incoming_tasks: FuturesUnordered<IncomingStreamHandlerFuture>,
    /// Subnetworks membership information
    membership: Membership,
    /// Pending blobs that need to be dispersed by `PeerId`
    to_sample: HashMap<PeerId, VecDeque<(Membership::NetworkId, BlobId)>>,
    /// Sample streams that has no tasks and should be closed.
    to_close: VecDeque<SampleStream>,
    /// Hook of pending samples channel
    samples_request_sender: UnboundedSender<(Membership::NetworkId, BlobId)>,
    /// Pending samples stream
    samples_request_stream: BoxStream<'static, (Membership::NetworkId, BlobId)>,
}

impl<Membership> SamplingBehaviour<Membership>
where
    Membership: MembershipHandler + 'static,
    Membership::NetworkId: Send,
{
    pub fn new(peer_id: PeerId, membership: Membership) -> Self {
        let stream_behaviour = libp2p_stream::Behaviour::new();
        let mut control = stream_behaviour.new_control();

        let incoming_streams = control
            .accept(SAMPLING_PROTOCOL)
            .expect("Just a single accept to protocol is valid");

        let outgoing_tasks = FuturesUnordered::new();
        let incoming_tasks = FuturesUnordered::new();

        let to_sample = HashMap::new();
        let to_close = VecDeque::new();

        let (samples_request_sender, receiver) = mpsc::unbounded_channel();
        let samples_request_stream = UnboundedReceiverStream::new(receiver).boxed();
        Self {
            peer_id,
            stream_behaviour,
            incoming_streams,
            control,
            outgoing_tasks,
            incoming_tasks,
            membership,
            to_sample,
            to_close,
            samples_request_sender,
            samples_request_stream,
        }
    }

    pub fn update_membership(&mut self, membership: Membership) {
        self.membership = membership;
    }

    /// Open a new stream from the underlying control to the provided peer
    async fn open_stream(
        peer_id: PeerId,
        mut control: Control,
    ) -> Result<SampleStream, SamplingError> {
        let stream = control
            .open_stream(peer_id, SAMPLING_PROTOCOL)
            .await
            .map_err(|error| SamplingError::OpenStream { peer_id, error })?;
        Ok(SampleStream { stream, peer_id })
    }

    /// Get a hook to the sender channel of the sample events
    pub fn sample_request_channel(&self) -> UnboundedSender<(Membership::NetworkId, BlobId)> {
        self.samples_request_sender.clone()
    }

    /// Task for handling streams, one message at a time
    /// Writes the request to the stream and waits for a response
    async fn stream_sample(
        mut stream: SampleStream,
        message: sampling::SampleRequest,
        subnetwork_id: SubnetworkId,
        blob_id: BlobId,
    ) -> Result<StreamWriterFutureSuccess, StreamWriterFutureError> {
        if let Err(error) = pack_to_writer(&message, &mut stream.stream).await {
            return Err((
                SamplingError::Io {
                    peer_id: stream.peer_id,
                    error,
                },
                Some(stream),
            ));
        }

        if let Err(error) = stream.stream.flush().await {
            return Err((
                SamplingError::Io {
                    peer_id: stream.peer_id,
                    error,
                },
                Some(stream),
            ));
        }

        let response = match unpack_from_reader(&mut stream.stream).await {
            Ok(response) => response,
            Err(error) => {
                return Err((
                    SamplingError::Io {
                        peer_id: stream.peer_id,
                        error,
                    },
                    Some(stream),
                ));
            }
        };

        // Safety: blob_id should always be a 32bytes hash
        Ok((blob_id, subnetwork_id, response, stream))
    }

    /// Get a pending outgoing request if its available
    fn next_request(
        peer_id: &PeerId,
        to_sample: &mut HashMap<PeerId, VecDeque<(SubnetworkId, BlobId)>>,
    ) -> Option<(SubnetworkId, BlobId)> {
        to_sample
            .get_mut(peer_id)
            .and_then(std::collections::VecDeque::pop_front)
    }

    /// Handle outgoing stream
    /// Schedule a new task if its available or drop the stream if not
    fn handle_outgoing_stream(
        outgoing_tasks: &FuturesUnordered<OutgoingStreamHandlerFuture>,
        to_sample: &mut HashMap<PeerId, VecDeque<(SubnetworkId, BlobId)>>,
        to_close: &mut VecDeque<SampleStream>,
        stream: SampleStream,
        cx: &Context<'_>,
    ) {
        let peer = stream.peer_id;

        // If there is a pending task schedule next one
        if let Some((subnetwork_id, blob_id)) = Self::next_request(&peer, to_sample) {
            let sample_request = sampling::SampleRequest::new(blob_id, subnetwork_id);
            outgoing_tasks
                .push(Self::stream_sample(stream, sample_request, subnetwork_id, blob_id).boxed());
            cx.waker().wake_by_ref();
        } else {
            // if not pop stream from connected ones
            to_close.push_back(stream);
        }
    }

    /// Handler for incoming streams
    /// Pulls a request from the stream and replies if possible
    async fn handle_incoming_stream(
        mut stream: SampleStream,
        channel: ResponseChannel,
    ) -> Result<SampleStream, (SamplingError, SampleStream)> {
        let request: sampling::SampleRequest = match unpack_from_reader(&mut stream.stream).await {
            Ok(req) => req,
            Err(error) => {
                return Err((
                    SamplingError::Io {
                        peer_id: stream.peer_id,
                        error,
                    },
                    stream,
                ))
            }
        };

        let request = match BehaviourSampleReq::try_from(request) {
            Ok(req) => req,
            Err(blob_id) => {
                return Err((
                    SamplingError::InvalidBlobId {
                        peer_id: stream.peer_id,
                        blob_id,
                    },
                    stream,
                ))
            }
        };

        if let Err(request) = channel.request_sender.send(request) {
            return Err((
                SamplingError::RequestChannel {
                    request,
                    peer_id: stream.peer_id,
                },
                stream,
            ));
        }

        let response: sampling::SampleResponse = match channel.response_receiver.await {
            Ok(resp) => resp.into(),
            Err(error) => {
                return Err((
                    SamplingError::ResponseChannel {
                        error,
                        peer_id: stream.peer_id,
                    },
                    stream,
                ))
            }
        };

        if let Err(error) = pack_to_writer(&response, &mut stream.stream).await {
            return Err((
                SamplingError::Io {
                    peer_id: stream.peer_id,
                    error,
                },
                stream,
            ));
        }

        if let Err(error) = stream.stream.flush().await {
            return Err((
                SamplingError::Io {
                    peer_id: stream.peer_id,
                    error,
                },
                stream,
            ));
        }

        Ok(stream)
    }

    /// Schedule an incoming stream to be replied
    /// Creates the necessary channels so requests can be replied from outside
    /// of this behaviour from whoever that takes the channels
    fn schedule_incoming_stream_task(
        incoming_tasks: &FuturesUnordered<IncomingStreamHandlerFuture>,
        sample_stream: SampleStream,
        cx: &Context<'_>,
    ) -> (Receiver<BehaviourSampleReq>, Sender<BehaviourSampleRes>) {
        let (request_sender, request_receiver) = oneshot::channel();
        let (response_sender, response_receiver) = oneshot::channel();
        let channel = ResponseChannel {
            request_sender,
            response_receiver,
        };
        incoming_tasks.push(Self::handle_incoming_stream(sample_stream, channel).boxed());
        // Scheduled a task, lets poll again.
        cx.waker().wake_by_ref();
        (request_receiver, response_sender)
    }
}

impl<Membership: MembershipHandler<Id = PeerId, NetworkId = SubnetworkId> + 'static>
    SamplingBehaviour<Membership>
{
    /// Schedule a new task for sample the blob, if stream is not available
    /// queue messages for later processing.
    fn sample(
        peer_id: PeerId,
        outgoing_tasks: &FuturesUnordered<OutgoingStreamHandlerFuture>,
        membership: &Membership,
        subnetwork_id: SubnetworkId,
        blob_id: BlobId,
        control: &Control,
    ) {
        let members = membership.members_of(&subnetwork_id);
        // TODO: peer selection for sampling should be randomly selected (?) filtering
        // ourselves currently we assume optimal setup which is one peer per
        // blob
        let peer = members
            .iter()
            .filter(|&id| id != &peer_id)
            .copied()
            .next()
            .expect("At least a single node should be a member of the subnetwork");
        // If its connected means we are already working on some other sample, enqueue
        // message, stream behaviour will dial peer if connection is not
        // present.
        let control = control.clone();
        let sample_request = sampling::SampleRequest::new(blob_id, subnetwork_id);
        let with_dial_task: OutgoingStreamHandlerFuture = async move {
            let stream = Self::open_stream(peer, control)
                .await
                .map_err(|err| (err, None))?;
            Self::stream_sample(stream, sample_request, subnetwork_id, blob_id).await
        }
        .boxed();
        outgoing_tasks.push(with_dial_task);
    }

    /// Auxiliary method that transforms a sample response into an event
    fn handle_sample_response(
        blob_id: BlobId,
        subnetwork_id: SubnetworkId,
        sample_response: sampling::SampleResponse,
        peer_id: PeerId,
    ) -> Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        match sample_response {
            sampling::SampleResponse::Error(error) => {
                Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::SamplingError {
                    error: SamplingError::Protocol {
                        subnetwork_id,
                        error,
                        peer_id,
                    },
                }))
            }
            sampling::SampleResponse::Blob(blob) => {
                Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::SamplingSuccess {
                    blob_id,
                    subnetwork_id,
                    light_blob: Box::new(blob.data),
                }))
            }
        }
    }
}

impl<M: MembershipHandler<Id = PeerId, NetworkId = SubnetworkId> + 'static> NetworkBehaviour
    for SamplingBehaviour<M>
{
    type ConnectionHandler = Either<
        <libp2p_stream::Behaviour as NetworkBehaviour>::ConnectionHandler,
        libp2p::swarm::dummy::ConnectionHandler,
    >;
    type ToSwarm = SamplingEvent;

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
        connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        role_override: Endpoint,
        port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        if !self.membership.is_allowed(&peer) {
            return Ok(Either::Right(libp2p::swarm::dummy::ConnectionHandler));
        }
        self.stream_behaviour
            .handle_established_outbound_connection(
                connection_id,
                peer,
                addr,
                role_override,
                port_use,
            )
            .map(Either::Left)
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
            peer_id,
            outgoing_tasks,
            incoming_tasks,
            to_sample,
            to_close,
            samples_request_stream,
            incoming_streams,
            membership,
            control,
            ..
        } = self;
        // poll pending outgoing samples
        if let Poll::Ready(Some((subnetwork_id, blob_id))) =
            samples_request_stream.poll_next_unpin(cx)
        {
            Self::sample(
                *peer_id,
                outgoing_tasks,
                membership,
                subnetwork_id,
                blob_id,
                control,
            );
        }
        // poll outgoing tasks
        if let Poll::Ready(Some(future_result)) = outgoing_tasks.poll_next_unpin(cx) {
            match future_result {
                Ok((blob_id, subnetwork_id, sample_response, stream)) => {
                    let peer_id = stream.peer_id;
                    // handle the free stream then return the success
                    Self::handle_outgoing_stream(outgoing_tasks, to_sample, to_close, stream, cx);
                    return Self::handle_sample_response(
                        blob_id,
                        subnetwork_id,
                        sample_response,
                        peer_id,
                    );
                }
                Err((error, stream)) => {
                    if let Some(stream) = stream {
                        to_close.push_back(stream);
                        cx.waker().wake_by_ref();
                    }
                    return Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::SamplingError {
                        error,
                    }));
                }
            }
        }
        // poll incoming streams
        if let Poll::Ready(Some((peer_id, stream))) = incoming_streams.poll_next_unpin(cx) {
            let sample_stream = SampleStream { stream, peer_id };
            let (request_receiver, response_sender) =
                Self::schedule_incoming_stream_task(incoming_tasks, sample_stream, cx);
            return Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::IncomingSample {
                request_receiver,
                response_sender,
            }));
        }
        // poll incoming tasks
        if let Poll::Ready(Some(res)) = incoming_tasks.poll_next_unpin(cx) {
            match res {
                Ok(sample_stream) => {
                    let (request_receiver, response_sender) =
                        Self::schedule_incoming_stream_task(incoming_tasks, sample_stream, cx);
                    return Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::IncomingSample {
                        request_receiver,
                        response_sender,
                    }));
                }
                //  Ignore `UnexpectedEof` errors and continue polling, stream is closed by the
                //  sender after the message is sent.
                Err((SamplingError::Io { error, .. }, stream))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    to_close.push_back(stream);
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Err((error, _)) => {
                    return Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::SamplingError {
                        error,
                    }))
                }
            }
        }
        // Deal with connection as the underlying behaviour would do
        if let Poll::Ready(ToSwarm::Dial { mut opts }) = self.stream_behaviour.poll(cx) {
            // attach known peer address if possible
            if let Some(address) = opts
                .get_peer_id()
                .and_then(|peer_id: PeerId| self.membership.get_address(&peer_id))
            {
                opts = DialOpts::peer_id(opts.get_peer_id().unwrap())
                    .addresses(vec![address])
                    .extend_addresses_through_behaviour()
                    .build();
                return Poll::Ready(ToSwarm::Dial { opts });
            }
        }
        // Discard stream, if still pending pushback to close later.
        if let Some(mut stream) = to_close.pop_front() {
            if stream.stream.close().poll_unpin(cx).is_pending() {
                to_close.push_back(stream);
                cx.waker().wake_by_ref();
            }
        }

        Poll::Pending
    }
}
