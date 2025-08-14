use std::{
    collections::{HashMap, VecDeque},
    task::{Context, Poll, Waker},
};

use either::Either;
use futures::{
    future::BoxFuture,
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
use libp2p_stream::Control;
use nomos_core::{block::BlockNumber, da::BlobId, header::HeaderId};
use nomos_da_messages::{sampling, sampling::SampleResponse};
use rand::{rngs::ThreadRng, seq::IteratorRandom as _};
use subnetworks_assignations::MembershipHandler;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::warn;

use crate::{
    addressbook::AddressBookHandler,
    protocol::SAMPLING_PROTOCOL,
    protocols::sampling::{
        connections::Connections,
        errors::SamplingError,
        historic::HistoricSamplingEvent,
        streams::{self, SampleStream},
        SampleFutureError, SampleResponseFutureSuccess, SubnetsConfig,
    },
    swarm::validator::SampleArgs,
    SubnetworkId,
};

// Context for tracking sampling requests with block information
#[derive(Debug, Clone)]
struct SamplingContext<Membership> {
    block_number: BlockNumber,
    block_id: HeaderId,
    peer_id: PeerId,
    subnetwork_id: SubnetworkId,
    blob_id: BlobId,
    membership: Membership,
}

// Future that includes context with the original sampling future result
type HistoricSamplingResponseFuture<Membership> = BoxFuture<
    'static,
    (
        SamplingContext<Membership>,
        Result<SampleResponseFutureSuccess, SampleFutureError>,
    ),
>;

/// Historic sampling protocol that uses membership snapshots
/// Takes care of sending sampling requests using provided historic membership
pub struct HistoricRequestSamplingBehaviour<Membership, Addressbook>
where
    Membership: MembershipHandler,
    Addressbook: AddressBookHandler,
{
    /// Self peer id
    local_peer_id: PeerId,
    /// Underlying stream behaviour
    stream_behaviour: libp2p_stream::Behaviour,
    /// Underlying stream control
    control: Control,
    /// Pending sampling stream tasks with their context
    stream_tasks: FuturesUnordered<HistoricSamplingResponseFuture<Membership>>,
    /// Pending blobs that need to be sampled from `PeerId` with context
    to_sample: HashMap<PeerId, VecDeque<(SamplingContext<Membership>, sampling::SampleRequest)>>,
    /// Queue of sampling contexts that still need a peer for sampling
    to_retry: VecDeque<SamplingContext<Membership>>,
    /// Sample streams that have no tasks and should be closed
    to_close: VecDeque<SampleStream>,
    /// Addressbook used for getting addresses of peers
    addressbook: Addressbook,
    /// Pending samples for historic requests sender
    historic_request_sender: UnboundedSender<SampleArgs<Membership>>,
    /// Pending samples for historic requests stream
    historic_request_stream: BoxStream<'static, SampleArgs<Membership>>,
    /// Subnets sampling config that is used when picking new subnetwork peers
    subnets_config: SubnetsConfig,
    /// Pending subnetwork connections for randomly selected peers
    connections: Connections,
    /// Waker for sampling polling
    waker: Option<Waker>,
}

impl<Membership, Addressbook> HistoricRequestSamplingBehaviour<Membership, Addressbook>
where
    Membership: MembershipHandler + Clone + Send + Sync + 'static,
    Membership::NetworkId: Send,
    Addressbook: AddressBookHandler + 'static,
{
    pub fn new(
        local_peer_id: PeerId,
        addressbook: Addressbook,
        subnets_config: SubnetsConfig,
    ) -> Self {
        let stream_behaviour = libp2p_stream::Behaviour::new();
        let control = stream_behaviour.new_control();

        let stream_tasks = FuturesUnordered::new();
        let to_sample = HashMap::new();
        let to_retry = VecDeque::new();
        let to_close = VecDeque::new();

        let (historic_request_sender, receiver) = mpsc::unbounded_channel();
        let historic_request_stream = UnboundedReceiverStream::new(receiver).boxed();

        let connections = Connections::new(subnets_config.shares_retry_limit);

        Self {
            local_peer_id,
            stream_behaviour,
            control,
            stream_tasks,
            to_sample,
            to_retry,
            to_close,
            addressbook,
            historic_request_sender,
            historic_request_stream,
            subnets_config,
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

    /// Get a hook to the sender channel for historic sampling requests
    pub fn historic_request_channel(&self) -> UnboundedSender<SampleArgs<Membership>> {
        self.historic_request_sender.clone()
    }

    pub fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

impl<Membership, Addressbook> HistoricRequestSamplingBehaviour<Membership, Addressbook>
where
    Membership:
        MembershipHandler<Id = PeerId, NetworkId = SubnetworkId> + Clone + Send + Sync + 'static,
    Addressbook: AddressBookHandler<Id = PeerId> + 'static,
{
    /// Schedule sampling tasks for blobs using the provided historic membership
    fn sample_historic(&mut self, sample_args: SampleArgs<Membership>) {
        let (blob_ids, block_number, block_id, membership) = sample_args;
        let mut rng = rand::thread_rng();

        for blob_id in blob_ids {
            // Get random subset of subnetworks for sampling
            let subnets: Vec<SubnetworkId> = (0..membership.last_subnetwork_id())
                .choose_multiple(&mut rng, self.subnets_config.num_of_subnets);

            // Sample shares from multiple subnetworks
            for subnetwork_id in &subnets {
                let membership = membership.clone();
                if let Some(peer_id) =
                    self.pick_subnetwork_peer(*subnetwork_id, &membership, &mut rng)
                {
                    let context = SamplingContext {
                        block_number,
                        block_id,
                        peer_id,
                        subnetwork_id: *subnetwork_id,
                        blob_id,
                        membership: membership.clone(),
                    };

                    let control = self.control.clone();
                    let sample_request =
                        sampling::SampleRequest::new_share(blob_id, *subnetwork_id);

                    let with_dial_task: HistoricSamplingResponseFuture<Membership> = async move {
                        // If we don't have an existing connection to the peer, this will
                        // immediately return `ConnectionReset` io error. To
                        // handle that, we need to queue `sample_request`
                        // for a peer and try again when the connection is
                        // established.
                        match Self::open_stream(peer_id, control).await {
                            Ok(stream) => {
                                let result = streams::stream_sample(stream, sample_request).await;
                                (context, result)
                            }
                            Err(err) => (context, Err((err, None))),
                            // todo: try to connect when the connection fails
                            // if the connection fails try up to 5 peers
                        }
                    }
                    .boxed();

                    self.stream_tasks.push(with_dial_task);
                    self.connections
                        .register_pending_peer(peer_id, *subnetwork_id);
                } else {
                    warn!("Subnetwork {subnetwork_id} has no peers for block {block_number}");
                }
            }
        }
    }

    // todo: sample commitments

    fn pick_subnetwork_peer(
        &self,
        subnetwork_id: SubnetworkId,
        membership: &Membership,
        rng: &mut ThreadRng,
    ) -> Option<PeerId> {
        let candidates = membership.members_of(&subnetwork_id);
        candidates
            .into_iter()
            .filter(|peer| *peer != self.local_peer_id)
            .choose(rng)
    }

    fn try_peer_sample_historic(&mut self, peer_id: PeerId) {
        if let Some((context, sample_request)) = self
            .to_sample
            .get_mut(&peer_id)
            .and_then(VecDeque::pop_front)
        {
            let control = self.control.clone();
            let open_stream_task: HistoricSamplingResponseFuture<Membership> = async move {
                // If we don't have an existing connection to the peer, this will
                // immediately return `ConnectionReset` io error. To
                // handle that, we need to queue `sample_request`
                // for a peer and try again when the connection is
                // established.
                match Self::open_stream(peer_id, control).await {
                    Ok(stream) => {
                        let result = streams::stream_sample(stream, sample_request).await;
                        (context, result)
                    }
                    Err(err) => (context, Err((err, None))),
                    // todo: try to connect when the connection fails
                    // if the connection fails try up to 5 peers
                }
            }
            .boxed();

            self.stream_tasks.push(open_stream_task);
            self.try_wake();
        }
    }

    fn try_subnetwork_sample_historic(&self, context: SamplingContext<Membership>) {
        if self.connections.should_retry(context.subnetwork_id) {
            let mut rng = rand::thread_rng();
            if let Some(peer_id) =
                self.pick_subnetwork_peer(context.subnetwork_id, &context.membership, &mut rng)
            {
                let control = self.control.clone();
                let sample_request =
                    sampling::SampleRequest::new_share(context.blob_id, context.subnetwork_id);
                let open_stream_task: HistoricSamplingResponseFuture<Membership> = async move {
                    // If we don't have an existing connection to the peer, this will
                    // immediately return `ConnectionReset` io error. To
                    // handle that, we need to queue `sample_request`
                    // for a peer and try again when the connection is
                    // established.
                    match Self::open_stream(peer_id, control).await {
                        Ok(stream) => (
                            context,
                            streams::stream_sample(stream, sample_request).await,
                        ),
                        Err(err) => (context, Err((err, None))),
                        // todo: try to connect when the connection fails
                        // if the connection fails try up to 5 peers
                    }
                }
                .boxed();
                self.stream_tasks.push(open_stream_task);
            } else {
                warn!(
                    "Subnetwork {} has no peers for block {}",
                    context.subnetwork_id, context.block_number
                );
            }
        }
    }

    /// Handle outgoing stream with block context
    fn schedule_outgoing_stream_task(&mut self, stream: SampleStream) {
        let peer_id = stream.peer_id;

        if let Some((context, sample_request)) = self
            .to_sample
            .get_mut(&peer_id)
            .and_then(VecDeque::pop_front)
        {
            let open_stream_task: HistoricSamplingResponseFuture<Membership> = async move {
                (
                    context,
                    streams::stream_sample(stream, sample_request).await,
                )
            }
            .boxed();

            self.stream_tasks.push(open_stream_task);
        } else {
            self.to_close.push_back(stream);
        }
    }

    /// Transform sample response into historic event with block context
    fn handle_sample_response(
        block_number: BlockNumber,
        block_id: HeaderId,
        sample_response: SampleResponse,
        peer_id: PeerId,
    ) -> Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        match sample_response {
            SampleResponse::Error(sampling::SampleError::Share(error)) => Poll::Ready(
                ToSwarm::GenerateEvent(HistoricSamplingEvent::SamplingError {
                    block_number,
                    block_id,
                    error: SamplingError::Share {
                        subnetwork_id: error.column_idx,
                        error,
                        peer_id,
                    },
                }),
            ),
            SampleResponse::Error(sampling::SampleError::Commitments(error)) => Poll::Ready(
                ToSwarm::GenerateEvent(HistoricSamplingEvent::SamplingError {
                    block_number,
                    block_id,
                    error: SamplingError::Commitments { error, peer_id },
                }),
            ),
            SampleResponse::Share(share) => Poll::Ready(ToSwarm::GenerateEvent(
                HistoricSamplingEvent::SamplingSuccess {
                    block_number,
                    block_id,
                    blob_id: share.blob_id,
                    subnetwork_id: share.data.share_idx,
                    light_share: Box::new(share.data),
                },
            )),
            SampleResponse::Commitments(commitments) => Poll::Ready(ToSwarm::GenerateEvent(
                HistoricSamplingEvent::CommitmentsSuccess {
                    block_number,
                    block_id,
                    blob_id: commitments.blob_id(),
                    commitments: Box::new(commitments),
                },
            )),
        }
    }

    fn handle_stream_response(
        &mut self,
        block_number: BlockNumber,
        block_id: HeaderId,
        peer_id: PeerId,
        sample_response: SampleResponse,
        stream: SampleStream,
    ) -> Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        self.schedule_outgoing_stream_task(stream);
        Self::handle_sample_response(block_number, block_id, sample_response, peer_id)
    }

    fn handle_stream_error(
        &mut self,
        block_number: BlockNumber,
        block_id: HeaderId,
        membership: Membership,
        error: SamplingError,
        maybe_stream: Option<SampleStream>,
    ) -> Option<Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>>> {
        match error {
            SamplingError::Io {
                error,
                peer_id,
                message: Some(sampling::SampleRequest::Share(message)),
            } if error.kind() == std::io::ErrorKind::ConnectionReset => {
                let subnetwork_id = message.share_idx;
                if !self.connections.should_retry(subnetwork_id) {
                    return Some(Poll::Ready(ToSwarm::GenerateEvent(
                        HistoricSamplingEvent::SamplingError {
                            block_number,
                            block_id,
                            error: SamplingError::NoSubnetworkPeers {
                                blob_id: message.blob_id,
                                subnetwork_id: message.share_idx,
                            },
                        },
                    )));
                }

                //todo: improve retries
                if self.connections.should_requeue(peer_id) {
                    self.to_retry.push_back(SamplingContext {
                        block_number,
                        block_id,
                        peer_id,
                        subnetwork_id,
                        blob_id: message.blob_id,
                        membership: membership.clone(),
                    });
                } else {
                    warn!("Peer {peer_id} has no more retries for subnetwork {subnetwork_id}");
                }

                if let Some(peer_queue) = self.to_sample.get_mut(&peer_id) {
                    peer_queue.push_back((
                        SamplingContext {
                            block_number,
                            block_id,
                            peer_id,
                            subnetwork_id: message.share_idx,
                            blob_id: message.blob_id,
                            membership,
                        },
                        sampling::SampleRequest::Share(message),
                    ));
                }

                if let Some(stream) = maybe_stream {
                    self.to_close.push_back(stream);
                }
                None
            }
            SamplingError::Io { error, .. }
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
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
                    HistoricSamplingEvent::SamplingError {
                        block_number,
                        block_id,
                        error,
                    },
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
            let (context, future_result) = future_result;
            match future_result {
                Ok((_peer_id, stream_response, stream)) => {
                    return Some(self.handle_stream_response(
                        context.block_number,
                        context.block_id,
                        context.peer_id,
                        stream_response,
                        stream,
                    ));
                }
                Err((error, maybe_stream)) => {
                    return self.handle_stream_error(
                        context.block_number,
                        context.block_id,
                        context.membership,
                        error,
                        maybe_stream,
                    );
                }
            }
        }
        None
    }
}

impl<Membership, Addressbook> NetworkBehaviour
    for HistoricRequestSamplingBehaviour<Membership, Addressbook>
where
    Membership:
        MembershipHandler<Id = PeerId, NetworkId = SubnetworkId> + Clone + Send + Sync + 'static,
    Addressbook: AddressBookHandler<Id = PeerId> + 'static,
{
    type ConnectionHandler = Either<
        <libp2p_stream::Behaviour as NetworkBehaviour>::ConnectionHandler,
        libp2p::swarm::dummy::ConnectionHandler,
    >;
    type ToSwarm = HistoricSamplingEvent;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        // Accept all connections for historic sampling since we don't have current
        // membership
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
        self.connections.register_connect(peer);
        self.try_peer_sample_historic(peer);
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

        // Poll pending historic sampling requests
        if let Poll::Ready(Some(sample_args)) = self.historic_request_stream.poll_next_unpin(cx) {
            self.sample_historic(sample_args);
        }

        // Handle retry queue
        if let Some(context) = self.to_retry.pop_front() {
            self.try_subnetwork_sample_historic(context);
        }

        // Poll stream tasks
        if let Some(result) = self.poll_stream_tasks(cx) {
            return result;
        }

        // Handle underlying stream behaviour
        if let Poll::Ready(ToSwarm::Dial { mut opts }) = self.stream_behaviour.poll(cx) {
            if let Some(address) = opts
                .get_peer_id()
                .and_then(|peer_id: PeerId| self.addressbook.get_address(&peer_id))
            {
                opts = DialOpts::peer_id(opts.get_peer_id().unwrap())
                    .addresses(vec![address])
                    .extend_addresses_through_behaviour()
                    .build();
                cx.waker().wake_by_ref();
                return Poll::Ready(ToSwarm::Dial { opts });
            }
        }

        // Close unused streams
        if let Some(mut stream) = self.to_close.pop_front() {
            if stream.stream.close().poll_unpin(cx).is_pending() {
                self.to_close.push_back(stream);
                cx.waker().wake_by_ref();
            }
        }

        Poll::Pending
    }
}
