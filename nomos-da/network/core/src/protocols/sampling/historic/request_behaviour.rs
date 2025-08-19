use std::{
    collections::HashSet,
    future::ready,
    io::ErrorKind,
    task::{Context, Poll},
};

use either::Either;
use futures::{
    future::BoxFuture,
    stream::{BoxStream, FuturesUnordered},
    AsyncWriteExt as _, FutureExt as _, StreamExt as _,
};
use kzgrs_backend::common::share::{DaLightShare, DaSharesCommitments};
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dial_opts::DialOpts, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler,
        THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};
use libp2p_stream::{Control, OpenStreamError};
use nomos_core::{da::BlobId, header::HeaderId};
use nomos_da_messages::sampling::{self, SampleResponse};
use rand::{rngs::ThreadRng, seq::IteratorRandom as _};
use subnetworks_assignations::MembershipHandler;
use thiserror::Error;
use tokio::{
    sync::mpsc::{self, UnboundedSender},
    time::{sleep, Duration},
};
use tokio_stream::wrappers::{BroadcastStream, UnboundedReceiverStream};

use crate::{
    addressbook::AddressBookHandler,
    protocol::SAMPLING_PROTOCOL,
    protocols::sampling::{
        errors::SamplingError,
        historic::{HistoricSamplingError, HistoricSamplingEvent},
        streams::{self, SampleStream},
        SubnetsConfig,
    },
    swarm::validator::SampleArgs,
    SubnetworkId,
};

const MAX_PEER_RETRIES: usize = 5;
const CONNECTION_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
enum StreamSamplingError {
    #[error("Connection timed out")]
    Timeout,
    #[error("Sampling error occurred: {0}")]
    SamplingError(#[from] SamplingError),
    #[error("Stream error occurred")]
    StreamError(Option<SampleStream>),
}

type HistoricSamplingResponseSuccess = (
    HeaderId,
    HashSet<DaLightShare>,
    HashSet<DaSharesCommitments>,
);

type HistoricFutureError = (HeaderId, HistoricSamplingError);

// Future that includes context with the original sampling future result
type HistoricSamplingResponseFuture =
    BoxFuture<'static, Result<HistoricSamplingResponseSuccess, HistoricFutureError>>;

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
    historic_request_tasks: FuturesUnordered<HistoricSamplingResponseFuture>,
    /// Addressbook used for getting addresses of peers
    addressbook: Addressbook,
    /// Pending samples for historic requests sender
    historic_request_sender: UnboundedSender<SampleArgs<Membership>>,
    /// Pending samples for historic requests stream
    historic_request_stream: BoxStream<'static, SampleArgs<Membership>>,
    /// Subnets sampling config that is used when picking new subnetwork peers
    subnets_config: SubnetsConfig,
    /// Broadcast channel for notifying about established connections
    connection_broadcast_sender: tokio::sync::broadcast::Sender<PeerId>,
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

        let (historic_request_sender, receiver) = mpsc::unbounded_channel();
        let historic_request_stream = UnboundedReceiverStream::new(receiver).boxed();

        let (connection_broadcast_sender, _) = tokio::sync::broadcast::channel(1024);

        Self {
            local_peer_id,
            stream_behaviour,
            control,
            historic_request_tasks: stream_tasks,
            addressbook,
            historic_request_sender,
            historic_request_stream,
            subnets_config,
            connection_broadcast_sender,
        }
    }

    // Open the stream and wait for connection
    async fn open_stream(
        peer_id: PeerId,
        mut control: Control,
        connection_receiver: tokio::sync::broadcast::Receiver<PeerId>,
    ) -> Result<SampleStream, StreamSamplingError> {
        let stream_result = control.open_stream(peer_id, SAMPLING_PROTOCOL).await;

        match stream_result {
            Ok(stream) => Ok(SampleStream { stream, peer_id }),
            Err(OpenStreamError::Io(io_err)) if io_err.kind() == ErrorKind::ConnectionReset => {
                let timer = sleep(CONNECTION_WAIT_TIMEOUT);
                tokio::pin!(timer);

                let mut connection_stream = BroadcastStream::new(connection_receiver)
                    .filter(|result| ready(matches!(result, Ok(pid) if *pid == peer_id)));

                tokio::select! {
                    Some(_) = connection_stream.next() => {
                        // Move on to retry
                    }
                    () = &mut timer => {
                        return Err(StreamSamplingError::Timeout);
                    }
                }

                // Retry once more after connection is established
                let stream = control
                    .open_stream(peer_id, SAMPLING_PROTOCOL)
                    .await
                    .map_err(|error| SamplingError::OpenStream { peer_id, error })?;

                Ok(SampleStream { stream, peer_id })
            }
            Err(error) => Err(StreamSamplingError::SamplingError(
                SamplingError::OpenStream { peer_id, error },
            )),
        }
    }

    /// Get a hook to the sender channel for historic sampling requests
    pub fn historic_request_channel(&self) -> UnboundedSender<SampleArgs<Membership>> {
        self.historic_request_sender.clone()
    }
}

impl<Membership, Addressbook> HistoricRequestSamplingBehaviour<Membership, Addressbook>
where
    Membership:
        MembershipHandler<Id = PeerId, NetworkId = SubnetworkId> + Clone + Send + Sync + 'static,
    Addressbook: AddressBookHandler<Id = PeerId> + Send + Sync + 'static,
{
    /// Schedule sampling tasks for blobs using the provided historic membership
    fn sample_historic(&self, sample_args: SampleArgs<Membership>) {
        let (blob_ids, _, block_id, membership) = sample_args;
        let control = self.control.clone();
        let mut rng = rand::thread_rng();
        let subnets: Vec<SubnetworkId> = (0..membership.last_subnetwork_id())
            .choose_multiple(&mut rng, self.subnets_config.num_of_subnets);
        let local_peer_id = self.local_peer_id;

        let connection_broadcast_sender = self.connection_broadcast_sender.clone();

        let request_future = async move {
            let commitments = Self::sample_all_commitments(
                &membership,
                &subnets,
                &local_peer_id,
                &blob_ids,
                &control,
                connection_broadcast_sender.subscribe(),
            )
            .await
            .map_err(|err| (block_id, err))?;

            let shares = Self::sample_all_shares(
                &subnets,
                &membership,
                &local_peer_id,
                &blob_ids,
                &control,
                &connection_broadcast_sender,
            )
            .await
            .map_err(|err| (block_id, err))?;

            Ok((block_id, shares, commitments))
        }
        .boxed();

        self.historic_request_tasks.push(request_future);
    }

    async fn sample_all_shares(
        subnets: &[SubnetworkId],
        membership: &Membership,
        local_peer_id: &PeerId,
        blob_ids: &HashSet<BlobId>,
        control: &Control,
        connection_sender: &tokio::sync::broadcast::Sender<PeerId>,
    ) -> Result<HashSet<DaLightShare>, HistoricSamplingError> {
        let mut subnetwork_tasks = FuturesUnordered::new();

        for subnetwork_id in subnets {
            let task = Self::sample_shares_for_subnetwork(
                membership,
                local_peer_id,
                control.clone(),
                blob_ids.clone(),
                *subnetwork_id,
                connection_sender.subscribe(),
            );
            subnetwork_tasks.push(task);
        }

        let mut all_shares = HashSet::new();
        while let Some(result) = subnetwork_tasks.next().await {
            match result {
                Ok(shares) => {
                    for share in shares {
                        all_shares.insert(share);
                    }
                }
                Err(err) => return Err(err),
            }
        }

        Ok(all_shares)
    }

    async fn sample_shares_for_subnetwork(
        membership: &Membership,
        local_peer_id: &PeerId,
        control: Control,
        mut blob_ids: HashSet<BlobId>,
        subnetwork_id: SubnetworkId,
        connection_receiver: tokio::sync::broadcast::Receiver<PeerId>,
    ) -> Result<Vec<DaLightShare>, HistoricSamplingError> {
        // Pre-select up to MAX_PEER_RETRIES peers from the subnetwork
        let candidate_peers = {
            let mut rng = rand::thread_rng();
            Self::pick_random_subnetwork_peers(subnetwork_id, membership, local_peer_id, &mut rng)
        };

        let mut all_shares = Vec::new();

        'peers_loop: for peer_id in candidate_peers {
            if blob_ids.is_empty() {
                break;
            }

            match Self::open_stream(peer_id, control.clone(), connection_receiver.resubscribe())
                .await
            {
                Ok(mut stream) => {
                    let blob_ids_to_try: Vec<BlobId> = blob_ids.iter().copied().collect();

                    for blob_id in blob_ids_to_try {
                        let request = sampling::SampleRequest::new_share(blob_id, subnetwork_id);
                        match Self::handle_share_request(stream, request).await {
                            Ok((share_data, new_stream)) => {
                                all_shares.push(share_data);
                                blob_ids.remove(&blob_id);
                                stream = new_stream;
                            }
                            Err(err) => {
                                if let StreamSamplingError::StreamError(Some(mut new_stream)) = err
                                {
                                    // try to send EOF to gracefully shutdown the stream
                                    let _ = new_stream.stream.close().await;
                                }

                                continue 'peers_loop; // Try next peer with
                                                      // remaining blobs
                            }
                        }
                    }

                    // close stream on success as well
                    let _ = stream.stream.close().await;
                }
                Err(err) => {
                    log::error!("Failed to open stream to peer {peer_id}: {err}");
                }
            }
        }

        if blob_ids.is_empty() {
            Ok(all_shares)
        } else {
            Err(HistoricSamplingError::SamplingFailed)
        }
    }

    async fn sample_all_commitments(
        membership: &Membership,
        subnets: &[SubnetworkId],
        local_peer_id: &PeerId,
        blob_ids: &HashSet<BlobId>,
        control: &Control,
        connection_receiver: tokio::sync::broadcast::Receiver<PeerId>,
    ) -> Result<HashSet<DaSharesCommitments>, HistoricSamplingError> {
        // Pre-select up to MAX_PEER_RETRIES peers from a random subnet
        let candidate_peers = {
            let mut peers = Vec::new();
            let mut rng = rand::thread_rng();

            for _ in 0..MAX_PEER_RETRIES {
                if let Some(subnet) = subnets.iter().choose(&mut rng) {
                    if let Some(peer) =
                        Self::pick_subnetwork_peer(*subnet, membership, local_peer_id, &mut rng)
                    {
                        peers.push(peer);
                    }
                }
            }
            peers
        };

        if candidate_peers.is_empty() {
            return Err(HistoricSamplingError::InternalServerError(
                "No peers available for commitments".to_owned(),
            ));
        }

        let mut remaining_blob_ids = blob_ids.clone();
        let mut commitments = HashSet::new();

        'peers_loop: for peer_id in candidate_peers {
            if remaining_blob_ids.is_empty() {
                break;
            }

            match Self::open_stream(peer_id, control.clone(), connection_receiver.resubscribe())
                .await
            {
                Ok(mut stream) => {
                    let blob_ids_to_try: Vec<BlobId> = remaining_blob_ids.iter().copied().collect();

                    for blob_id in blob_ids_to_try {
                        let request = sampling::SampleRequest::new_commitments(blob_id);
                        match Self::handle_commitment_request(stream, request).await {
                            Ok((commitment, new_stream)) => {
                                commitments.insert(commitment);
                                remaining_blob_ids.remove(&blob_id);
                                stream = new_stream;
                            }
                            Err(err) => {
                                if let StreamSamplingError::StreamError(Some(mut new_stream)) = err
                                {
                                    // try to send EOF to gracefully shutdown the stream
                                    let _ = new_stream.stream.close().await;
                                }

                                continue 'peers_loop;
                            }
                        }
                    }

                    // close stream on success as well
                    let _ = stream.stream.close().await;
                }
                Err(err) => {
                    log::error!("Failed to open stream to peer {peer_id}: {err}");
                }
            }
        }

        if remaining_blob_ids.is_empty() {
            Ok(commitments)
        } else {
            Err(HistoricSamplingError::SamplingFailed)
        }
    }

    async fn handle_share_request(
        stream: SampleStream,
        request: sampling::SampleRequest,
    ) -> Result<(DaLightShare, SampleStream), StreamSamplingError> {
        let (_, response_result, new_stream) = streams::stream_sample(stream, request)
            .await
            .map_err(|(_, maybe_stream)| StreamSamplingError::StreamError(maybe_stream))?;

        if let SampleResponse::Share(share_data) = response_result {
            Ok((share_data.data, new_stream))
        } else {
            Err(StreamSamplingError::StreamError(Some(new_stream)))
        }
    }

    async fn handle_commitment_request(
        stream: SampleStream,
        request: sampling::SampleRequest,
    ) -> Result<(DaSharesCommitments, SampleStream), StreamSamplingError> {
        let (_, response_result, new_stream) = streams::stream_sample(stream, request)
            .await
            .map_err(|(_, maybe_stream)| StreamSamplingError::StreamError(maybe_stream))?;

        if let SampleResponse::Commitments(comm) = response_result {
            Ok((comm, new_stream))
        } else {
            Err(StreamSamplingError::StreamError(Some(new_stream)))
        }
    }

    fn pick_subnetwork_peer(
        subnetwork_id: SubnetworkId,
        membership: &Membership,
        local_peer_id: &PeerId,
        rng: &mut ThreadRng,
    ) -> Option<PeerId> {
        let candidates = membership.members_of(&subnetwork_id);
        candidates
            .into_iter()
            .filter(|peer| peer != local_peer_id)
            .choose(rng)
    }

    fn pick_random_subnetwork_peers(
        subnetwork_id: SubnetworkId,
        membership: &Membership,
        local_peer_id: &PeerId,
        rng: &mut ThreadRng,
    ) -> Vec<PeerId> {
        let candidates = membership.members_of(&subnetwork_id);
        let available_peers: Vec<_> = candidates
            .into_iter()
            .filter(|peer| peer != local_peer_id)
            .collect();

        available_peers
            .into_iter()
            .choose_multiple(rng, MAX_PEER_RETRIES)
    }

    fn poll_historic_tasks(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>>> {
        if let Poll::Ready(Some(future_result)) = self.historic_request_tasks.poll_next_unpin(cx) {
            cx.waker().wake_by_ref();
            match future_result {
                Ok((block_id, shares, commitments)) => {
                    return Some(Self::handle_historic_success(block_id, shares, commitments));
                }
                Err((block_id, sampling_error)) => {
                    return Some(Self::handle_historic_error(block_id, sampling_error));
                }
            }
        }
        None
    }

    const fn handle_historic_success(
        block_id: HeaderId,
        shares: HashSet<DaLightShare>,
        commitments: HashSet<DaSharesCommitments>,
    ) -> Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        Poll::Ready(ToSwarm::GenerateEvent(
            HistoricSamplingEvent::SamplingSuccess {
                block_id,
                commitments,
                shares,
            },
        ))
    }

    const fn handle_historic_error(
        block_id: HeaderId,
        sampling_error: HistoricSamplingError,
    ) -> Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        match sampling_error {
            HistoricSamplingError::SamplingFailed
            | HistoricSamplingError::InternalServerError(_) => Poll::Ready(ToSwarm::GenerateEvent(
                HistoricSamplingEvent::SamplingError {
                    block_id,
                    error: sampling_error,
                },
            )),
        }
    }
}

impl<Membership, Addressbook> NetworkBehaviour
    for HistoricRequestSamplingBehaviour<Membership, Addressbook>
where
    Membership:
        MembershipHandler<Id = PeerId, NetworkId = SubnetworkId> + Clone + Send + Sync + 'static,
    Addressbook: AddressBookHandler<Id = PeerId> + Send + Sync + 'static,
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
        let _ = self.connection_broadcast_sender.send(peer);

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
        let _ = self.connection_broadcast_sender.send(peer);

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
        // Poll pending historic sampling requests
        if let Poll::Ready(Some(sample_args)) = self.historic_request_stream.poll_next_unpin(cx) {
            self.sample_historic(sample_args);
        }

        // poll stream tasks
        if let Some(result) = self.poll_historic_tasks(cx) {
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

        Poll::Pending
    }
}
