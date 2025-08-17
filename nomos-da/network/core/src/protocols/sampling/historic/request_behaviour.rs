use std::task::{Context, Poll, Waker};

use either::Either;
use futures::{
    future::BoxFuture,
    stream::{BoxStream, FuturesUnordered},
    FutureExt as _, StreamExt as _,
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
use libp2p_stream::Control;
use nomos_core::header::HeaderId;
use nomos_da_messages::sampling::{self, SampleResponse};
use rand::{rngs::ThreadRng, seq::IteratorRandom as _};
use subnetworks_assignations::MembershipHandler;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::{
    addressbook::AddressBookHandler,
    protocol::SAMPLING_PROTOCOL,
    protocols::sampling::{
        errors::SamplingError,
        historic::HistoricSamplingEvent,
        streams::{self, SampleStream},
        SubnetsConfig,
    },
    swarm::validator::SampleArgs,
    SubnetworkId,
};

type HistoricSamplingResponseSuccess = (HeaderId, Vec<DaLightShare>);

type HistoricFutureError = (HeaderId, SamplingError);

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

        let (historic_request_sender, receiver) = mpsc::unbounded_channel();
        let historic_request_stream = UnboundedReceiverStream::new(receiver).boxed();

        Self {
            local_peer_id,
            stream_behaviour,
            control,
            historic_request_tasks: stream_tasks,
            addressbook,
            historic_request_sender,
            historic_request_stream,
            subnets_config,
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
    Addressbook: AddressBookHandler<Id = PeerId> + Send + Sync + 'static,
{
    /// Schedule sampling tasks for blobs using the provided historic membership
    fn sample_historic(&self, sample_args: SampleArgs<Membership>) {
        let (blob_ids, _, block_id, membership) = sample_args;

        let control = self.control.clone();
        let mut rng = rand::thread_rng();
        let subnets: Vec<SubnetworkId> = (0..membership.last_subnetwork_id())
            .choose_multiple(&mut rng, self.subnets_config.num_of_subnets);

        let mut shares = Vec::new();
        let peer_id = self.local_peer_id;

        let request_future = async move {
            let mut subnetwork_tasks = Vec::new();

            for subnetwork_id in subnets {
                let mut rng = rand::thread_rng();
                let peer_id =
                    Self::pick_subnetwork_peer(subnetwork_id, &membership, &peer_id, &mut rng)
                        .unwrap();

                let control = control.clone();
                let blob_ids = blob_ids.clone();

                // create a task per subnetwork that will sample all blobs for the same
                // subnetwork (peer) re-using the same stream for single peerid
                let task = async move {
                    let mut shares: Vec<DaLightShare> = Vec::new();

                    // todo: dial new connections and pick new peer on retry, up to 5 times
                    let mut stream = Self::open_stream(peer_id, control).await.unwrap();

                    for blob_id in &blob_ids {
                        let sample_request =
                            sampling::SampleRequest::new_share(*blob_id, subnetwork_id);

                        //todo: handle error response
                        let response = streams::stream_sample(stream, sample_request)
                            .await
                            .unwrap();

                        stream = response.2;

                        match response.1 {
                            SampleResponse::Share(share) => {
                                shares.push(share.data);
                            }
                            SampleResponse::Commitments(_) => {
                                todo!("implement commitments")
                            }
                            SampleResponse::Error(_) => todo!("handle error"),
                        }
                    }

                    (subnetwork_id, shares)
                };

                subnetwork_tasks.push(task);
            }

            for task in subnetwork_tasks {
                let (_, task_shares) = task.await;
                shares.extend(task_shares);
            }

            Ok((block_id, shares))
        }
        .boxed();

        self.historic_request_tasks.push(request_future);
    }

    // todo: sample commitments

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

    fn poll_historic_tasks(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>>> {
        if let Poll::Ready(Some(future_result)) = self.historic_request_tasks.poll_next_unpin(cx) {
            cx.waker().wake_by_ref();
            match future_result {
                Ok((block_id, shares)) => {
                    return Some(Self::handle_historic_response(block_id, shares));
                }
                Err((block_id, sampling_error)) => {
                    return Some(Self::handle_historic_error(block_id, sampling_error));
                }
            }
        }
        None
    }

    fn handle_historic_response(
        block_id: HeaderId,
        shares: Vec<DaLightShare>,
    ) -> Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        //todo: implement commitemtns
        let commitments = Box::new(DaSharesCommitments::default());
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
        sampling_error: SamplingError,
    ) -> Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        Poll::Ready(ToSwarm::GenerateEvent(
            HistoricSamplingEvent::SamplingError {
                block_id,
                error: sampling_error,
            },
        ))
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
        self.waker = Some(cx.waker().clone());

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
