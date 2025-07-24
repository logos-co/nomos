use std::{
    collections::{HashMap, VecDeque},
    pin::Pin,
    task::{Context, Poll, Waker},
};

use either::Either;
use futures::{
    channel::{
        oneshot,
        oneshot::{Receiver, Sender},
    },
    stream::{BoxStream, FuturesUnordered},
    AsyncWriteExt as _, FutureExt as _, StreamExt as _,
};
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dial_opts::DialOpts, ConnectionDenied, ConnectionId, DialFailure, FromSwarm,
        NetworkBehaviour, THandler, THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};
use libp2p_stream::{Control, IncomingStreams};
use nomos_core::da::BlobId;
use nomos_da_messages::sampling;
use rand::{rngs::ThreadRng, seq::IteratorRandom as _};
use subnetworks_assignations::MembershipHandler;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::warn;

use super::{
    connections::Connections,
    errors::SamplingError,
    streams::{self, SampleStream},
    BehaviourSampleReq, BehaviourSampleRes, ResponseChannel, SampleStreamResponse, SamplingEvent,
    SamplingStreamFuture, SubnetsConfig,
};
use crate::{addressbook::AddressBookHandler, protocol::SAMPLING_PROTOCOL, SubnetworkId};

type AttemptNumber = usize;

/// Executor sampling protocol
/// Takes care of sending and replying sampling requests
pub struct SamplingBehaviour<Membership, Addressbook>
where
    Membership: MembershipHandler,
    Addressbook: AddressBookHandler,
{
    /// Self peer id
    local_peer_id: PeerId,
    /// Underlying stream behaviour
    stream_behaviour: libp2p_stream::Behaviour,
    /// Incoming sample request streams
    incoming_streams: IncomingStreams,
    /// Underlying stream control
    control: Control,
    /// Pending sampling stream tasks
    stream_tasks: FuturesUnordered<SamplingStreamFuture>,
    /// Pending blobs that need to be sampled from `PeerId`
    to_sample: HashMap<PeerId, VecDeque<sampling::SampleRequest>>,
    /// Queue of blobs that still needs a peer for sampling.
    to_retry: VecDeque<(Membership::NetworkId, BlobId)>,
    /// Sample streams that has no tasks and should be closed.
    to_close: VecDeque<SampleStream>,
    /// Subnetworks membership information
    membership: Membership,
    /// Addressbook used for getting addresses of peers
    addressbook: Addressbook,
    /// Peers that were selected for
    sampling_peers: HashMap<SubnetworkId, PeerId>,
    /// Pending samples for shares sender
    shares_request_sender: UnboundedSender<BlobId>,
    /// Pending samples for shares stream
    shares_request_stream: BoxStream<'static, BlobId>,
    /// Pending samples for commitments sender
    commitments_request_sender: UnboundedSender<BlobId>,
    /// Pending samples for commitments stream
    commitments_request_stream: BoxStream<'static, BlobId>,
    /// Commitments that are waiting for response and the attemt count
    commitments_requests: HashMap<BlobId, AttemptNumber>,
    /// Subnets sampling config that is used when picking new subnetwork peers.
    subnets_config: SubnetsConfig,
    /// Refresh signal stream that triggers the subnetwork list refresh in
    /// sampling baheviour.
    subnet_refresh_signal: Pin<Box<dyn futures::Stream<Item = ()> + Send>>,
    /// Pending subnetwork connections for randomly selected peers after refresh
    /// interval.
    connections: Connections,
    /// Waker for sampling polling
    waker: Option<Waker>,
}

impl<Membership, Addressbook> SamplingBehaviour<Membership, Addressbook>
where
    Membership: MembershipHandler + 'static,
    Membership::NetworkId: Send,
    Addressbook: AddressBookHandler + 'static,
{
    pub fn new(
        local_peer_id: PeerId,
        membership: Membership,
        addressbook: Addressbook,
        subnets_config: SubnetsConfig,
        refresh_signal: impl futures::Stream<Item = ()> + Send + 'static,
    ) -> Self {
        let stream_behaviour = libp2p_stream::Behaviour::new();
        let mut control = stream_behaviour.new_control();

        let incoming_streams = control
            .accept(SAMPLING_PROTOCOL)
            .expect("Just a single accept to protocol is valid");

        let stream_tasks = FuturesUnordered::new();
        let to_sample = HashMap::new();
        let to_retry = VecDeque::new();
        let to_close = VecDeque::new();

        let sampling_peers = HashMap::new();
        let (shares_request_sender, receiver) = mpsc::unbounded_channel();
        let shares_request_stream = UnboundedReceiverStream::new(receiver).boxed();

        let (commitments_request_sender, receiver) = mpsc::unbounded_channel();
        let commitments_request_stream = UnboundedReceiverStream::new(receiver).boxed();
        let commitments_requests = HashMap::new();

        let subnet_refresh_signal = Box::pin(refresh_signal);
        let connections = Connections::new(subnets_config.shares_retry_limit);

        Self {
            local_peer_id,
            stream_behaviour,
            incoming_streams,
            control,
            stream_tasks,
            to_sample,
            to_retry,
            to_close,
            membership,
            addressbook,
            sampling_peers,
            shares_request_sender,
            shares_request_stream,
            commitments_request_sender,
            commitments_request_stream,
            commitments_requests,
            subnets_config,
            subnet_refresh_signal,
            connections,
            waker: None,
        }
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

    /// Get a hook to the sender channel of the share events
    pub fn shares_request_channel(&self) -> UnboundedSender<BlobId> {
        self.shares_request_sender.clone()
    }

    /// Get a hook to the sender channel of the commitments events
    pub fn commitments_request_channel(&self) -> UnboundedSender<BlobId> {
        self.commitments_request_sender.clone()
    }

    /// Schedule an incoming stream to be replied
    /// Creates the necessary channels so requests can be replied from outside
    /// of this behaviour from whoever that takes the channels
    fn schedule_incoming_stream_task(
        &self,
        sample_stream: SampleStream,
    ) -> (Receiver<BehaviourSampleReq>, Sender<BehaviourSampleRes>) {
        let (request_sender, request_receiver) = oneshot::channel();
        let (response_sender, response_receiver) = oneshot::channel();
        let channel = ResponseChannel {
            request_sender,
            response_receiver,
        };
        self.stream_tasks
            .push(streams::handle_incoming_stream(sample_stream, channel).boxed());
        // Scheduled a task, lets poll again.
        (request_receiver, response_sender)
    }

    pub fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

impl<Membership, Addressbook> SamplingBehaviour<Membership, Addressbook>
where
    Membership: MembershipHandler<Id = PeerId, NetworkId = SubnetworkId> + 'static,
    Addressbook: AddressBookHandler<Id = PeerId> + 'static,
{
    /// Schedule a new task for sample the blob, if stream is not available
    /// queue messages for later processing.
    fn sample_share(&mut self, blob_id: BlobId) {
        for (subnetwork_id, peer_id) in &self.sampling_peers {
            let subnetwork_id = *subnetwork_id;
            let peer_id = *peer_id;
            let control = self.control.clone();
            // If its connected means we are already working on some other sample, enqueue
            // message, stream behaviour will dial peer if connection is not
            // present.
            let sample_request = sampling::SampleRequest::new_share(blob_id, subnetwork_id);
            let with_dial_task: SamplingStreamFuture = async move {
                // If we don't have an existing connection to the peer, this will immediately
                // return `ConnectionReset` io error. To handle that, we need to queue
                // `sample_request` for a peer and try again when the connection is
                // established.
                let stream = Self::open_stream(peer_id, control)
                    .await
                    .map_err(|err| (err, None))?;
                streams::stream_sample(stream, sample_request).await
            }
            .boxed();
            self.stream_tasks.push(with_dial_task);
            self.connections
                .register_pending_peer(peer_id, subnetwork_id);
        }
    }

    fn sample_commitments(&mut self, blob_id: BlobId) {
        if let Some((_, &peer_id)) = &self.sampling_peers.iter().choose(&mut rand::thread_rng()) {
            let control = self.control.clone();
            let sample_request = sampling::SampleRequest::new_commitments(blob_id);
            let with_dial_task: SamplingStreamFuture = async move {
                let stream = Self::open_stream(peer_id, control)
                    .await
                    .map_err(|err| (err, None))?;
                streams::stream_sample(stream, sample_request).await
            }
            .boxed();
            self.commitments_requests
                .entry(blob_id)
                .and_modify(|count| *count += 1)
                .or_insert(1);
            self.stream_tasks.push(with_dial_task);
        }
    }

    fn try_peer_sample_share(&mut self, peer_id: PeerId) {
        if let Some(sample_request) = self
            .to_sample
            .get_mut(&peer_id)
            .and_then(VecDeque::pop_front)
        {
            let control = self.control.clone();
            let open_stream_task: SamplingStreamFuture = async move {
                let stream = Self::open_stream(peer_id, control)
                    .await
                    .map_err(|err| (err, None))?;
                streams::stream_sample(stream, sample_request).await
            }
            .boxed();
            self.stream_tasks.push(open_stream_task);
            self.try_wake();
        }
    }

    fn try_subnetwork_sample_share(&mut self, blob_id: BlobId, subnetwork_id: SubnetworkId) {
        if self.connections.should_retry(subnetwork_id) {
            let mut rng = rand::thread_rng();
            if let Some(peer_id) = self.pick_subnetwork_peer(subnetwork_id, &mut rng) {
                let control = self.control.clone();
                let sample_request = sampling::SampleRequest::new_share(blob_id, subnetwork_id);
                let open_stream_task: SamplingStreamFuture = async move {
                    let stream = Self::open_stream(peer_id, control)
                        .await
                        .map_err(|err| (err, None))?;
                    streams::stream_sample(stream, sample_request).await
                }
                .boxed();
                self.stream_tasks.push(open_stream_task);
                self.sampling_peers.insert(subnetwork_id, peer_id);
            } else {
                warn!("Subnetwork {subnetwork_id} has no peers");
            }
        }
    }

    fn refresh_subnets(&mut self) {
        // Previously selected subnetworks and their peers won't be used anymore.
        self.sampling_peers.clear();
        self.connections.clear();

        let mut rng = rand::thread_rng();
        let subnets: Vec<SubnetworkId> = (0..self.membership.last_subnetwork_id())
            .choose_multiple(&mut rng, self.subnets_config.num_of_subnets);

        // Chosing a random peer for a subnetwork, even if previously selected peer for
        // different subnetwork might also be a member of another subnetwork.
        for subnetwork_id in subnets {
            if let Some(peer_id) = self.pick_subnetwork_peer(subnetwork_id, &mut rng) {
                self.sampling_peers.insert(subnetwork_id, peer_id);
            } else {
                warn!("Subnetwork {subnetwork_id} has no peers");
            }
        }
    }

    fn pick_subnetwork_peer(
        &self,
        subnetwork_id: SubnetworkId,
        rng: &mut ThreadRng,
    ) -> Option<PeerId> {
        let candidates = self.membership.members_of(&subnetwork_id);
        candidates
            .into_iter()
            .filter(|peer| *peer != self.local_peer_id)
            .choose(rng)
    }

    /// Handle outgoing stream
    /// Schedule a new task if its available or drop the stream if not
    fn schedule_outgoing_stream_task(&mut self, stream: SampleStream) {
        let peer_id = stream.peer_id;

        // If there is a pending task schedule next one
        if let Some(sample_request) = self
            .to_sample
            .get_mut(&peer_id)
            .and_then(VecDeque::pop_front)
        {
            self.stream_tasks
                .push(streams::stream_sample(stream, sample_request).boxed());
        } else {
            // if not pop stream from connected ones
            self.to_close.push_back(stream);
        }
    }

    /// Auxiliary method that transforms a sample response into an event
    fn handle_sample_response(
        sample_response: sampling::SampleResponse,
        peer_id: PeerId,
    ) -> Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        match sample_response {
            sampling::SampleResponse::Error(sampling::SampleError::Share(error)) => {
                Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::SamplingError {
                    error: SamplingError::Share {
                        subnetwork_id: error.column_idx,
                        error,
                        peer_id,
                    },
                }))
            }
            sampling::SampleResponse::Error(sampling::SampleError::Commitments(error)) => {
                Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::SamplingError {
                    error: SamplingError::Commitments { error, peer_id },
                }))
            }
            sampling::SampleResponse::Share(share) => {
                Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::SamplingSuccess {
                    blob_id: share.blob_id,
                    subnetwork_id: share.data.share_idx,
                    light_share: Box::new(share.data),
                }))
            }
            sampling::SampleResponse::Commitments(commitments) => {
                Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::CommitmentsSuccess {
                    blob_id: commitments.blob_id(),
                    commitments: Box::new(commitments),
                }))
            }
        }
    }

    fn handle_stream_response(
        &mut self,
        peer_id: PeerId,
        stream_response: SampleStreamResponse,
        stream: SampleStream,
    ) -> Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        match stream_response {
            SampleStreamResponse::Writer(sample_response) => {
                // Handle the free stream then return the response.
                self.schedule_outgoing_stream_task(stream);
                Self::handle_sample_response(*sample_response, peer_id)
            }
            SampleStreamResponse::Reader => {
                // Writer might behoping to send to this stream another request, wait
                // until the writer closes the stream.
                let (request_receiver, response_sender) =
                    self.schedule_incoming_stream_task(stream);
                Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::IncomingSample {
                    request_receiver,
                    response_sender,
                }))
            }
        }
    }

    fn handle_stream_error(
        &mut self,
        error: SamplingError,
        maybe_stream: Option<SampleStream>,
    ) -> Option<Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>>> {
        match error {
            SamplingError::Io {
                error,
                peer_id,
                message: Some(sampling::SampleRequest::Share(message)),
            } if error.kind() == std::io::ErrorKind::ConnectionReset => {
                // Propagate error for blob_id if retry limit is reached.
                if !self.connections.should_retry(message.share_idx) {
                    return Some(Poll::Ready(ToSwarm::GenerateEvent(
                        SamplingEvent::no_subnetwork_peers_err(message.blob_id, message.share_idx),
                    )));
                }
                // Dial to peer failed, should requeue to different peer.
                if self.connections.should_requeue(peer_id) {
                    self.to_retry
                        .push_back((message.share_idx, message.blob_id));
                }
                // Connection reset with the attached message comes from the stream that we've
                // tried to write to - if connection reset happens during the write it's most
                // likely because we didn't have the connection to peer or peer closed stream on
                // it's end because it stopped waiting for messages through this stream.
                if let Some(peer_queue) = self.to_sample.get_mut(&peer_id) {
                    peer_queue.push_back(sampling::SampleRequest::Share(message));
                }
                // Stream is useless if connection was reset.
                if let Some(stream) = maybe_stream {
                    self.to_close.push_back(stream);
                }
                None
            }
            SamplingError::Io { error, .. }
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                // Eof is actually expected and is proper signal about remote closing the
                // stream. Do not propagate and continue execution of behaviour poll method.
                if let Some(stream) = maybe_stream {
                    self.to_close.push_back(stream);
                }
                None
            }
            error => {
                if let Some(stream) = maybe_stream {
                    self.to_close.push_back(stream);
                }
                Some(Poll::Ready(ToSwarm::GenerateEvent(
                    SamplingEvent::SamplingError { error },
                )))
            }
        }
    }

    fn poll_stream_tasks(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>>> {
        if let Poll::Ready(Some(future_result)) = self.stream_tasks.poll_next_unpin(cx) {
            cx.waker().wake_by_ref();
            match future_result {
                Ok((peer_id, stream_response, stream)) => {
                    return Some(self.handle_stream_response(peer_id, stream_response, stream));
                }
                Err((error, maybe_stream)) => {
                    return self.handle_stream_error(error, maybe_stream);
                }
            }
        }
        None
    }
}

impl<Membership, Addressbook> NetworkBehaviour for SamplingBehaviour<Membership, Addressbook>
where
    Membership: MembershipHandler<Id = PeerId, NetworkId = SubnetworkId> + 'static,
    Addressbook: AddressBookHandler<Id = PeerId> + 'static,
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
        self.connections.register_connect(peer);
        self.try_peer_sample_share(peer);
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
        if let FromSwarm::DialFailure(DialFailure {
            peer_id: Some(peer_id),
            ..
        }) = event
        {
            self.connections.register_disconnect(peer_id);
        }
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
        self.waker = Some(cx.waker().clone());

        // Check if a new set of subnets and peers need to be selected.
        if self.subnet_refresh_signal.poll_next_unpin(cx) == Poll::Ready(Some(())) {
            self.refresh_subnets();
        }

        // poll pending outgoing samples for shares
        if let Poll::Ready(Some(blob_id)) = self.shares_request_stream.poll_next_unpin(cx) {
            self.sample_share(blob_id);
        }

        // poll pending outgoing samples for commitments
        if let Poll::Ready(Some(blob_id)) = self.commitments_request_stream.poll_next_unpin(cx) {
            self.sample_commitments(blob_id);
        }

        if let Some((subnetwork_id, blob_id)) = self.to_retry.pop_front() {
            self.try_subnetwork_sample_share(blob_id, subnetwork_id);
        }

        // poll incoming streams
        if let Poll::Ready(Some((peer_id, stream))) = self.incoming_streams.poll_next_unpin(cx) {
            let sample_stream = SampleStream { stream, peer_id };
            let (request_receiver, response_sender) =
                self.schedule_incoming_stream_task(sample_stream);
            cx.waker().wake_by_ref();
            return Poll::Ready(ToSwarm::GenerateEvent(SamplingEvent::IncomingSample {
                request_receiver,
                response_sender,
            }));
        }

        // poll stream tasks
        if let Some(result) = self.poll_stream_tasks(cx) {
            return result;
        }

        // Deal with connection as the underlying behaviour would do
        if let Poll::Ready(ToSwarm::Dial { mut opts }) = self.stream_behaviour.poll(cx) {
            // attach known peer address if possible
            if let Some(address) = opts
                .get_peer_id()
                .and_then(|peer_id: PeerId| self.addressbook.get_address(&peer_id))
            {
                opts = DialOpts::peer_id(opts.get_peer_id().unwrap())
                    .addresses(vec![address])
                    .extend_addresses_through_behaviour()
                    .build();
                // If we dial, some outgoing task is created, poll again.
                cx.waker().wake_by_ref();
                return Poll::Ready(ToSwarm::Dial { opts });
            }
        }

        // Discard stream, if still pending pushback to close later.
        if let Some(mut stream) = self.to_close.pop_front() {
            if stream.stream.close().poll_unpin(cx).is_pending() {
                self.to_close.push_back(stream);
                cx.waker().wake_by_ref();
            }
        }

        Poll::Pending
    }
}
