use std::{collections::HashSet, io, marker::PhantomData, time::Duration};

use futures::{
    stream::{self, FuturesUnordered},
    StreamExt as _,
};
use libp2p::{
    core::transport::ListenerId,
    identity::Keypair,
    swarm::{DialError, SwarmEvent},
    Multiaddr, PeerId, Swarm, SwarmBuilder, TransportError,
};
use nomos_core::{block::SessionNumber, da::BlobId, header::HeaderId};
use nomos_da_messages::replication::ReplicationRequest;
use subnetworks_assignations::MembershipHandler;
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedSender},
    time::interval,
};
use tokio_stream::wrappers::{IntervalStream, UnboundedReceiverStream};
use tracing::debug;

use super::{
    common::handlers::{handle_validator_dispersal_event, ValidationTask},
    DispersalValidatorEvent,
};
use crate::{
    addressbook::AddressBookHandler,
    behaviour::validator::{ValidatorBehaviour, ValidatorBehaviourEvent},
    maintenance::{balancer::ConnectionBalancerCommand, monitor::ConnectionMonitorCommand},
    protocols::{
        dispersal::validator::behaviour::DispersalEvent,
        replication::behaviour::{ReplicationConfig, ReplicationEvent},
        sampling::{SamplingEvent, SubnetsConfig},
    },
    swarm::{
        common::{
            handlers::{handle_replication_event, handle_sampling_event, monitor_event},
            monitor::{DAConnectionMonitorSettings, MonitorEvent},
            policy::DAConnectionPolicy,
        },
        BalancerStats, ConnectionBalancer, ConnectionMonitor, DAConnectionPolicySettings,
        MonitorStats,
    },
    SubnetworkId,
};

pub type SampleArgs<Membership> = (HashSet<BlobId>, SessionNumber, HeaderId, Membership);

// Metrics
const EVENT_SAMPLING: &str = "sampling";
const EVENT_VALIDATOR_DISPERSAL: &str = "validator_dispersal";
const EVENT_REPLICATION: &str = "replication";

type ValidatorSwarmType<Membership, HistoricMembership, Addressbook> = Swarm<
    ValidatorBehaviour<
        ConnectionBalancer<Membership>,
        ConnectionMonitor<Membership>,
        Membership,
        HistoricMembership,
        Addressbook,
    >,
>;

pub struct SwarmSettings {
    pub policy_settings: DAConnectionPolicySettings,
    pub monitor_settings: DAConnectionMonitorSettings,
    pub balancer_interval: Duration,
    pub redial_cooldown: Duration,
    pub replication_settings: ReplicationConfig,
    pub subnets_settings: SubnetsConfig,
}

pub struct ValidatorEventsStream {
    pub sampling_events_receiver: UnboundedReceiverStream<SamplingEvent>,
    pub validation_events_receiver: UnboundedReceiverStream<DispersalValidatorEvent>,
}

pub struct ValidatorSwarm<Membership, HistoricMembership, Addressbook>
where
    Membership:
        MembershipHandler<NetworkId = SubnetworkId, Id = PeerId> + Clone + Send + Sync + 'static,
    HistoricMembership:
        MembershipHandler<NetworkId = SubnetworkId, Id = PeerId> + Clone + Send + Sync + 'static,
    Addressbook: AddressBookHandler<Id = PeerId> + Clone + Send + Sync + 'static,
{
    swarm: ValidatorSwarmType<Membership, HistoricMembership, Addressbook>,
    sampling_events_sender: UnboundedSender<SamplingEvent>,
    validation_events_sender: UnboundedSender<DispersalValidatorEvent>,
    membership: Membership,
    phantom: PhantomData<HistoricMembership>,
}

impl<Membership, HistoricMembership, Addressbook>
    ValidatorSwarm<Membership, HistoricMembership, Addressbook>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId> + Clone + Send + Sync,
    HistoricMembership:
        MembershipHandler<NetworkId = SubnetworkId, Id = PeerId> + Clone + Send + Sync,
    Addressbook: AddressBookHandler<Id = PeerId> + Clone + Send + Sync + 'static,
{
    pub fn new(
        key: Keypair,
        membership: Membership,
        addressbook: Addressbook,
        SwarmSettings {
            policy_settings,
            monitor_settings,
            balancer_interval,
            redial_cooldown,
            replication_settings: replication_config,
            subnets_settings: subnets_config,
        }: SwarmSettings,
        refresh_signal: impl futures::Stream<Item = ()> + Send + 'static,
    ) -> (Self, ValidatorEventsStream) {
        let (sampling_events_sender, sampling_events_receiver) = unbounded_channel();
        let (validation_events_sender, validation_events_receiver) = unbounded_channel();

        let sampling_events_receiver = UnboundedReceiverStream::new(sampling_events_receiver);
        let validation_events_receiver = UnboundedReceiverStream::new(validation_events_receiver);
        let local_peer_id = PeerId::from_public_key(&key.public());

        let policy = DAConnectionPolicy::new(policy_settings, membership.clone(), local_peer_id);
        let monitor = ConnectionMonitor::new(monitor_settings, policy.clone());
        let balancer_interval_stream = if balancer_interval.is_zero() {
            stream::pending().boxed() // Stream that never produces items
        } else {
            IntervalStream::new(interval(balancer_interval))
                .map(|_| ())
                .boxed()
        };
        let balancer = ConnectionBalancer::new(
            local_peer_id,
            membership.clone(),
            policy,
            balancer_interval_stream,
        );

        tracing::info!("DA validator peer_id: {local_peer_id}");

        (
            Self {
                swarm: Self::build_swarm(
                    key,
                    membership.clone(),
                    addressbook,
                    balancer,
                    monitor,
                    redial_cooldown,
                    replication_config,
                    subnets_config,
                    refresh_signal,
                ),
                sampling_events_sender,
                validation_events_sender,
                membership,
                phantom: PhantomData,
            },
            ValidatorEventsStream {
                sampling_events_receiver,
                validation_events_receiver,
            },
        )
    }
    fn build_swarm(
        key: Keypair,
        membership: Membership,
        addressbook: Addressbook,
        balancer: ConnectionBalancer<Membership>,
        monitor: ConnectionMonitor<Membership>,
        redial_cooldown: Duration,
        replication_config: ReplicationConfig,
        subnets_config: SubnetsConfig,
        refresh_signal: impl futures::Stream<Item = ()> + Send + 'static,
    ) -> ValidatorSwarmType<Membership, HistoricMembership, Addressbook> {
        SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_quic()
            .with_behaviour(|key| {
                ValidatorBehaviour::new(
                    key,
                    membership,
                    addressbook,
                    balancer,
                    monitor,
                    redial_cooldown,
                    replication_config,
                    subnets_config,
                    refresh_signal,
                )
            })
            .expect("Validator behaviour should build")
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX))
            })
            .build()
    }

    pub fn dial(&mut self, addr: Multiaddr) -> Result<(), DialError> {
        self.swarm.dial(addr)?;
        Ok(())
    }

    pub fn listen_on(
        &mut self,
        address: Multiaddr,
    ) -> Result<ListenerId, TransportError<io::Error>> {
        self.swarm.listen_on(address)
    }

    pub fn shares_request_channel(&mut self) -> UnboundedSender<BlobId> {
        self.swarm
            .behaviour()
            .sampling_behaviour()
            .shares_request_channel()
    }

    pub fn historic_sample_request_channel(
        &mut self,
    ) -> UnboundedSender<SampleArgs<HistoricMembership>> {
        self.swarm
            .behaviour()
            .sampling_behaviour()
            .historical_request_channel()
    }

    pub fn commitments_request_channel(&mut self) -> UnboundedSender<BlobId> {
        self.swarm
            .behaviour()
            .sampling_behaviour()
            .commitments_request_channel()
    }

    pub fn balancer_command_channel(
        &mut self,
    ) -> UnboundedSender<ConnectionBalancerCommand<BalancerStats>> {
        self.swarm
            .behaviour()
            .balancer_behaviour()
            .command_channel()
    }

    pub fn monitor_command_channel(
        &mut self,
    ) -> UnboundedSender<ConnectionMonitorCommand<MonitorStats>> {
        self.swarm.behaviour().monitor_behavior().command_channel()
    }

    pub fn local_peer_id(&self) -> &PeerId {
        self.swarm.local_peer_id()
    }

    pub const fn protocol_swarm(
        &self,
    ) -> &ValidatorSwarmType<Membership, HistoricMembership, Addressbook> {
        &self.swarm
    }

    pub const fn protocol_swarm_mut(
        &mut self,
    ) -> &mut ValidatorSwarmType<Membership, HistoricMembership, Addressbook> {
        &mut self.swarm
    }

    fn handle_sampling_event(&mut self, event: SamplingEvent) {
        monitor_event(
            self.swarm.behaviour_mut().monitor_behaviour_mut(),
            MonitorEvent::from(&event),
        );
        handle_sampling_event(&self.sampling_events_sender, event);
    }

    fn handle_dispersal_event(&mut self, event: DispersalEvent) -> Option<ValidationTask> {
        monitor_event(
            self.swarm.behaviour_mut().monitor_behaviour_mut(),
            MonitorEvent::from(&event),
        );
        handle_validator_dispersal_event(&self.validation_events_sender, event)
    }

    fn handle_replication_event(&mut self, event: ReplicationEvent) {
        monitor_event(
            self.swarm.behaviour_mut().monitor_behaviour_mut(),
            MonitorEvent::from(&event),
        );
        handle_replication_event(
            &self.validation_events_sender,
            &self.membership,
            self.local_peer_id(),
            event,
        );
    }

    fn handle_behaviour_event(
        &mut self,
        validation_tasks: &FuturesUnordered<ValidationTask>,
        event: ValidatorBehaviourEvent<
            ConnectionBalancer<Membership>,
            ConnectionMonitor<Membership>,
            Membership,
            HistoricMembership,
            Addressbook,
        >,
    ) {
        match event {
            ValidatorBehaviourEvent::Sampling(event) => {
                tracing::info!(
                    counter.behaviour_events_received = 1,
                    event = EVENT_SAMPLING
                );
                self.handle_sampling_event(event);
            }
            ValidatorBehaviourEvent::Dispersal(event) => {
                tracing::info!(
                    counter.behaviour_events_received = 1,
                    event = EVENT_VALIDATOR_DISPERSAL,
                    share_size = event.share_size()
                );
                if let Some(task) = self.handle_dispersal_event(event) {
                    validation_tasks.push(Box::pin(task));
                }
            }
            ValidatorBehaviourEvent::Replication(event) => {
                tracing::info!(
                    counter.behaviour_events_received = 1,
                    event = EVENT_REPLICATION,
                    share_size = event.share_size()
                );
                self.handle_replication_event(event);
            }
            _ => {}
        }
    }

    #[expect(clippy::cognitive_complexity, reason = "Multiple DA protocol streams")]
    pub async fn run(mut self) {
        let mut validation_tasks = FuturesUnordered::new();
        loop {
            tokio::select! {
                Some(event) = self.swarm.next() => {
                    debug!("Da swarm event received: {event:?}");
                    match event {
                        SwarmEvent::Behaviour(behaviour_event) => {
                            self.handle_behaviour_event(&validation_tasks, behaviour_event);
                        }
                        SwarmEvent::ConnectionEstablished { .. }
                        | SwarmEvent::ConnectionClosed { .. }
                        | SwarmEvent::IncomingConnection { .. }
                        | SwarmEvent::IncomingConnectionError { .. }
                        | SwarmEvent::OutgoingConnectionError { .. }
                        | SwarmEvent::NewListenAddr { .. }
                        | SwarmEvent::ExpiredListenAddr { .. }
                        | SwarmEvent::ListenerClosed { .. }
                        | SwarmEvent::ListenerError { .. }
                        | SwarmEvent::Dialing { .. }
                        | SwarmEvent::NewExternalAddrCandidate { .. }
                        | SwarmEvent::ExternalAddrConfirmed { .. }
                        | SwarmEvent::ExternalAddrExpired { .. }
                        | SwarmEvent::NewExternalAddrOfPeer { .. } => {}
                        event => {
                            debug!("Unsupported validator swarm event: {event:?}");
                        }
                    }
                }
                Some((validation_result, event)) = validation_tasks.next() => {
                    if let Err(e) = validation_result {
                        tracing::error!("Validation task failed for event {event:?}: {e:?}");
                        continue;
                    }

                    let replication_behaviour =
                        self.swarm.behaviour_mut().replication_behaviour_mut();

                    match event {
                        DispersalEvent::IncomingShare(share) => {
                            replication_behaviour.send_message(&ReplicationRequest::from(*share));
                        }
                        DispersalEvent::IncomingTx(signed_mantle_tx) => {
                            replication_behaviour.send_message(&ReplicationRequest::from(*signed_mantle_tx.1));
                        }
                        DispersalEvent::DispersalError { .. } => {}
                    }
                }
            }
        }
    }
}
