use crate::protocol::DISPERSAL_PROTOCOL;
use crate::SubnetworkId;
use either::Either;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::{AsyncWriteExt, FutureExt, StreamExt};
use libp2p::core::Endpoint;
use libp2p::swarm::{
    ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent,
    THandlerOutEvent, ToSwarm,
};
use libp2p::{Multiaddr, PeerId, Stream};
use libp2p_stream::IncomingStreams;
use log::debug;
use nomos_da_messages::dispersal::dispersal_res::MessageType;
use nomos_da_messages::dispersal::{DispersalReq, DispersalRes};
use nomos_da_messages::{pack_message, unpack_from_reader};
use std::io::Error;
use std::task::{Context, Poll};
use subnetworks_assignations::MembershipHandler;

#[derive(Debug)]
pub enum DispersalEvent {
    /// Received a n
    IncomingMessage { message: DispersalReq },
}
pub struct DispersalValidatorBehaviour<Membership> {
    stream_behaviour: libp2p_stream::Behaviour,
    incoming_streams: IncomingStreams,
    tasks: FuturesUnordered<BoxFuture<'static, Result<(DispersalReq, Stream), Error>>>,
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
    /// This task handles a single message receive. Then it writes up the acknowledgment into the same
    /// stream as response and finish.
    async fn handle_new_stream(mut stream: Stream) -> Result<(DispersalReq, Stream), Error> {
        let message: DispersalReq = unpack_from_reader(&mut stream).await?;
        let blob_id = message.blob.clone().unwrap().blob_id;
        let response = DispersalRes {
            message_type: Some(MessageType::BlobId(blob_id)),
        };
        let message_bytes = pack_message(&response)?;
        stream.write_all(&message_bytes).await?;
        stream.flush().await?;
        Ok((message, stream))
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
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(Either::Right(libp2p::swarm::dummy::ConnectionHandler))
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
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
        let Self {
            incoming_streams,
            tasks,
            ..
        } = self;
        match tasks.poll_next_unpin(cx) {
            Poll::Ready(Some(Ok((message, stream)))) => {
                tasks.push(Self::handle_new_stream(stream).boxed());
                return Poll::Ready(ToSwarm::GenerateEvent(DispersalEvent::IncomingMessage {
                    message,
                }));
            }
            Poll::Ready(Some(Err(error))) => {
                debug!("Error on dispersal stream {error:?}");
            }
            _ => {}
        }
        if let Poll::Ready(Some((_peer_id, stream))) = incoming_streams.poll_next_unpin(cx) {
            tasks.push(Self::handle_new_stream(stream).boxed());
        }
        // TODO: probably must be smarter when to wake this
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address_book::AddressBook;
    use crate::protocols::dispersal::executor::behaviour::{
        DispersalExecutorBehaviour, DispersalExecutorEvent,
    };
    use crate::test_utils::Indicator;
    use kzgrs_backend::common::blob::DaBlob;
    use kzgrs_backend::common::ColumnIndex;
    use kzgrs_backend::encoder::{DaEncoder, DaEncoderParams, EncodedData};
    use libp2p::identity::Keypair;
    use libp2p::swarm::SwarmEvent;
    use libp2p::{identity, quic, PeerId, Swarm};
    use nomos_core::da::DaEncoder as TraitDaEncoder;
    use nomos_da_messages::common::Blob;
    use rand::RngCore;
    use std::collections::{HashMap, HashSet};
    use tokio::sync::mpsc::UnboundedSender;
    use tracing::warn;
    use tracing_subscriber::fmt::TestWriter;
    use tracing_subscriber::EnvFilter;

    #[derive(Clone, Debug)]
    struct Neighbourhood {
        pub membership: HashMap<PeerId, HashSet<SubnetworkId>>,
    }

    impl MembershipHandler for Neighbourhood {
        type NetworkId = SubnetworkId;
        type Id = PeerId;

        fn membership(&self, peer_id: &PeerId) -> HashSet<Self::NetworkId> {
            self.membership.get(peer_id).cloned().unwrap_or_default()
        }

        fn members_of(&self, subnetwork: &Self::NetworkId) -> HashSet<Self::Id> {
            self.membership
                .iter()
                .filter_map(|(id, nets)| {
                    if nets.contains(subnetwork) {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .collect()
        }

        fn is_allowed(&self, _id: &Self::Id) -> bool {
            self.membership.contains_key(_id)
        }

        fn members(&self) -> HashSet<Self::Id> {
            let mut members = HashSet::new();
            for id in self.membership.keys() {
                members.insert(*id);
            }
            members
        }
    }

    fn executor_swarm(
        addressbook: AddressBook,
        key: Keypair,
        peer_id: PeerId,
        membership: impl MembershipHandler<NetworkId = u32, Id = PeerId> + 'static,
    ) -> libp2p::Swarm<
        DispersalExecutorBehaviour<impl MembershipHandler<NetworkId = u32, Id = PeerId>>,
    > {
        libp2p::SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_other_transport(|keypair| quic::tokio::Transport::new(quic::Config::new(keypair)))
            .unwrap()
            .with_behaviour(|_key| {
                DispersalExecutorBehaviour::new(peer_id, membership, addressbook)
            })
            .unwrap()
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(std::time::Duration::from_secs(u64::MAX))
            })
            .build()
    }

    fn validator_swarm(
        key: Keypair,
        membership: impl MembershipHandler<NetworkId = u32, Id = PeerId> + 'static,
    ) -> libp2p::Swarm<
        DispersalValidatorBehaviour<impl MembershipHandler<NetworkId = u32, Id = PeerId>>,
    > {
        libp2p::SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_other_transport(|keypair| quic::tokio::Transport::new(quic::Config::new(keypair)))
            .unwrap()
            .with_behaviour(|_key| DispersalValidatorBehaviour::new(membership))
            .unwrap()
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(std::time::Duration::from_secs(u64::MAX))
            })
            .build()
    }

    fn prepare_swarm_config(
        num_instances: usize,
        group_id: usize,
    ) -> Vec<(Keypair, PeerId, Multiaddr)> {
        let mut configs = Vec::with_capacity(num_instances);

        for i in 0..num_instances {
            let keypair = identity::Keypair::generate_ed25519();
            let peer_id = PeerId::from(keypair.public());
            let port = 5200 + group_id * num_instances + i;
            let addr: Multiaddr = format!("/ip4/127.0.0.1/udp/{port}/quic-v1")
                .parse()
                .unwrap();

            configs.push((keypair, peer_id, addr));
        }
        configs
    }

    fn create_membership(
        num_instances: usize,
        subnets: usize,
        peer_ids: &[PeerId],
    ) -> Neighbourhood {
        let membership = peer_ids
            .iter()
            .take(num_instances)
            .enumerate()
            .map(|(i, &peer_id)| {
                let subnet_id = (i * subnets / num_instances) as SubnetworkId;
                (peer_id, HashSet::from([subnet_id]))
            })
            .collect();

        Neighbourhood { membership }
    }

    async fn run_executor_swarm(
        mut swarm: Swarm<
            DispersalExecutorBehaviour<
                impl MembershipHandler<NetworkId = u32, Id = PeerId> + Sized + 'static,
            >,
        >,
        indicator: Indicator,
    ) -> usize {
        let mut msg_counter = 0;

        loop {
            if let Some(true) = indicator.receive() {
                warn!("Executor terminated");
                return msg_counter;
            }
            let event = swarm.next().await;
            debug!("Executor event: {event:?}");
            match event {
                Some(SwarmEvent::Behaviour(DispersalExecutorEvent::DispersalSuccess {
                    ..
                })) => {
                    msg_counter += 1;
                }
                Some(SwarmEvent::Behaviour(DispersalExecutorEvent::DispersalError { error })) => {
                    debug!("Dispersal Error: {error}");
                }
                _ => {}
            }
        }
    }

    async fn run_validator_swarm(
        mut swarm: Swarm<
            DispersalValidatorBehaviour<
                impl MembershipHandler<NetworkId = u32, Id = PeerId> + Sized + 'static,
            >,
        >,
        messages_to_expect: usize,
        own_indicator: Indicator,
        exec_indicator: Indicator,
        val_indicators: Vec<Indicator>,
    ) -> (usize, usize) {
        let (mut msg_0_counter, mut msg_1_counter) = (0, 0);

        loop {
            if msg_0_counter == messages_to_expect || msg_1_counter == messages_to_expect {
                for _ in 0..10 {
                    own_indicator.send(true);
                }
                let mut flag = false;

                for indicator in &val_indicators {
                    if let Some(true) = indicator.receive() {
                        flag = true;
                    } else {
                        flag = false;
                        break;
                    }
                }
                if flag {
                    debug!("Breaking Validator"); 
                    exec_indicator.send(true);
                    for _ in 0..10 {
                        own_indicator.send(true);
                    }
                    break;
                }
            } else if let Some(event) = swarm.next().await {
                debug!("Validator event: {event:?}");

                if let SwarmEvent::Behaviour(DispersalEvent::IncomingMessage { message }) = event {
                    debug!("Validator received blob: {message:?}");

                    // Check data has structure and content as expected
                    if let Some(Blob { blob_id, data }) = message.blob {
                        let deserialized_blob: DaBlob = bincode::deserialize(&data).unwrap();
                        assert_eq!(blob_id, deserialized_blob.id());
                        if message.subnetwork_id == 0 {
                            msg_0_counter += 1;
                        } else {
                            msg_1_counter += 1;
                        }
                    }
                }
            }
        }
        (msg_0_counter, msg_1_counter)
    }

    fn send_dispersal_messages(
        disperse_blob_sender: UnboundedSender<(u32, DaBlob)>,
        subnet_id: u32,
        messages_to_send: usize,
    ) {
        // Generate 16 blobs
        let mut buff = vec![0; 32 * DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE];
        rand::thread_rng().fill_bytes(&mut buff);
        let params = DaEncoderParams::default_with(32);
        let encoder = DaEncoder::new(params);
        let encoded_data: EncodedData = encoder.encode(&buff).unwrap();
        let blobs: Vec<DaBlob> = encoded_data
            .chunked_data
            .columns()
            .enumerate()
            .map(|(idx, column)| DaBlob {
                column,
                column_idx: idx as ColumnIndex,
                column_commitment: Default::default(),
                aggregated_column_commitment: Default::default(),
                aggregated_column_proof: Default::default(),
                rows_commitments: vec![],
                rows_proofs: vec![],
            })
            .collect();

        for i in 0..messages_to_send {
            debug!("Sending blob {i} to subnet {subnet_id} ...");
            disperse_blob_sender
                .send((subnet_id, blobs[i].clone()))
                .unwrap();
        }
    }

    #[tokio::test]
    async fn test_validation_behaviour() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .compact()
            .with_writer(TestWriter::default())
            .try_init();

        const ALL_INSTANCES: usize = 20;
        const MESSAGES_TO_SEND: usize = 10;
        const GROUPS: usize = 4;

        let executor_0_config = prepare_swarm_config(ALL_INSTANCES / GROUPS, 0);
        let validator_0_config = prepare_swarm_config(ALL_INSTANCES / GROUPS, 1);

        let executor_1_config = prepare_swarm_config(ALL_INSTANCES / GROUPS, 2);
        let validator_1_config = prepare_swarm_config(ALL_INSTANCES / GROUPS, 3);

        let all_ids = executor_0_config
            .iter()
            .chain(validator_0_config.iter())
            .chain(executor_1_config.iter())
            .chain(validator_1_config.iter())
            .map(|(_, peer_id, _)| *peer_id)
            .collect::<Vec<_>>();

        let to_p2p_address = |(_, peer_id, addr): &(_, PeerId, Multiaddr)| {
            (*peer_id, addr.clone().with_p2p(*peer_id).unwrap())
        };

        // Including only validators in the addressbook is for testing purposes only.
        // In real-world scenarios, the addressbook should include all types of swarms.
        let addressbook = AddressBook::from_iter(
            validator_0_config
                .iter()
                .chain(validator_1_config.iter())
                .map(to_p2p_address),
        );

        let neighbours = create_membership(ALL_INSTANCES, 2, &all_ids);

        // Create swarms
        let mut executor_0_swarms: Vec<_> = vec![];
        let mut validator_0_swarms: Vec<_> = vec![];
        let mut executor_1_swarms: Vec<_> = vec![];
        let mut validator_1_swarms: Vec<_> = vec![];

        for i in 0..ALL_INSTANCES / GROUPS {
            let (k, executor_peer, _) = executor_0_config[i].clone();
            let (k2, _, _) = validator_0_config[i].clone();
            let (k3, executor_peer2, _) = executor_1_config[i].clone();
            let (k4, _, _) = validator_1_config[i].clone();

            let executor_0 =
                executor_swarm(addressbook.clone(), k, executor_peer, neighbours.clone());
            let validator_0 = validator_swarm(k2, neighbours.clone());
            executor_0_swarms.push(executor_0);
            validator_0_swarms.push(validator_0);

            let executor_1 =
                executor_swarm(addressbook.clone(), k3, executor_peer2, neighbours.clone());
            let validator_1 = validator_swarm(k4, neighbours.clone());
            executor_1_swarms.push(executor_1);
            validator_1_swarms.push(validator_1);
        }

        // Let validator swarms to listen
        for i in 0..ALL_INSTANCES / GROUPS {
            let (_, _, mut addr) = validator_0_config[i].clone();
            validator_0_swarms[i].listen_on(addr).unwrap();

            (_, _, addr) = validator_1_config[i].clone();
            validator_1_swarms[i].listen_on(addr).unwrap();
        }

        // Collect blob message senders from executors
        let mut message_senders: Vec<UnboundedSender<(u32, DaBlob)>> = Vec::new();

        for i in 0..ALL_INSTANCES / GROUPS {
            let blob_sender_0 = executor_0_swarms[i].behaviour().blobs_sender();
            let blob_sender_1 = executor_1_swarms[i].behaviour().blobs_sender();
            message_senders.extend(vec![blob_sender_0, blob_sender_1]);
        }

        let mut executor_tasks = vec![];
        let executor_indicator = Indicator::new(MESSAGES_TO_SEND);
        let validator_indicators = vec![Indicator::new(MESSAGES_TO_SEND); ALL_INSTANCES / 2];

        // Spawn executors
        for i in (0..ALL_INSTANCES / GROUPS).rev() {
            let (indicator_0, indicator_1) =
                (executor_indicator.clone(), executor_indicator.clone());

            let swarm = executor_0_swarms.remove(i);
            let executor_0_poll = async move { run_executor_swarm(swarm, indicator_0).await };

            let swarm = executor_1_swarms.remove(i);
            let executor_1_poll = async move { run_executor_swarm(swarm, indicator_1).await };

            executor_tasks.extend(vec![
                tokio::spawn(executor_0_poll),
                tokio::spawn(executor_1_poll),
            ]);
        }

        let mut validator_tasks = vec![];
        // Spawn validators
        for i in (0..ALL_INSTANCES / GROUPS).rev() {
            let (indicator_0, indicator_1) =
                (executor_indicator.clone(), executor_indicator.clone());
            let (clone1, clone2) = (validator_indicators.clone(), validator_indicators.clone());
            let (own_indicator_0, own_indicator_1) = (
                validator_indicators[2 * i].clone(),
                validator_indicators[2 * i + 1].clone(),
            );

            let swarm = validator_0_swarms.remove(i);
            let validator_0_poll = async {
                run_validator_swarm(
                    swarm,
                    MESSAGES_TO_SEND,
                    own_indicator_0,
                    indicator_0,
                    clone1,
                )
                .await
            };

            let swarm = validator_1_swarms.remove(i);
            let validator_1_poll = async {
                run_validator_swarm(
                    swarm,
                    MESSAGES_TO_SEND,
                    own_indicator_1,
                    indicator_1,
                    clone2,
                )
                .await
            };

            validator_tasks.extend(vec![
                tokio::spawn(validator_0_poll),
                tokio::spawn(validator_1_poll),
            ]);
        }

        let t1 = tokio::spawn(async move {
            let mut sent_messages = 0;
            for task in executor_tasks {
                sent_messages += task.await.unwrap();
                debug!("Executor task received success",);
            }
            sent_messages
        });

        let t2 = tokio::spawn(async move {
            let mut dispersal_request_counter = (0usize, 0usize);

            for task in validator_tasks {
                let requested = task.await.unwrap();
                debug!(
                    "Validator received {:?} messages from subnet 0",
                    requested.0
                );
                debug!(
                    "Validator received {:?} messages from subnet 1\n",
                    requested.1
                );
                dispersal_request_counter = (
                    dispersal_request_counter.0 + requested.0,
                    dispersal_request_counter.1 + requested.1,
                );
            }

            dispersal_request_counter
        });
        // Send messages from all executors
        for i in (0..ALL_INSTANCES / 2).rev() {
            let sender = message_senders.remove(i);
            send_dispersal_messages(sender, if i % 2 == 0 { 0 } else { 1 }, MESSAGES_TO_SEND);
        }

        let sent_messages = t1.await.unwrap();
        let dispersal_request_counter = t2.await.unwrap();

        // Check dispersed and confirmed equal to sent messages
        assert_eq!(
            sent_messages,
            MESSAGES_TO_SEND * (ALL_INSTANCES / GROUPS) * 2
        );
        // Check received per subnet are half of all messages sent
        assert_eq!(
            dispersal_request_counter.0,
            MESSAGES_TO_SEND * (ALL_INSTANCES / GROUPS)
        );
        assert_eq!(
            dispersal_request_counter.1,
            MESSAGES_TO_SEND * (ALL_INSTANCES / GROUPS)
        );
    }
}
