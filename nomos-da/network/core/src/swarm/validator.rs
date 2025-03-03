use std::{io, time::Duration};

use futures::{stream, StreamExt};
use kzgrs_backend::common::blob::DaBlob;
use libp2p::{
    core::transport::ListenerId,
    identity::Keypair,
    swarm::{DialError, SwarmEvent},
    Multiaddr, PeerId, Swarm, SwarmBuilder, TransportError,
};
use log::debug;
use nomos_core::da::BlobId;
use subnetworks_assignations::MembershipHandler;
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedSender},
    time::interval,
};
use tokio_stream::wrappers::{IntervalStream, UnboundedReceiverStream};

use super::ConnectionBalancer;
use crate::{
    address_book::AddressBook,
    behaviour::validator::{ValidatorBehaviour, ValidatorBehaviourEvent},
    protocols::{
        dispersal::validator::behaviour::DispersalEvent, replication::behaviour::ReplicationEvent,
        sampling::behaviour::SamplingEvent,
    },
    swarm::{
        common::{
            handlers::{
                handle_replication_event, handle_sampling_event, handle_validator_dispersal_event,
                monitor_event,
            },
            monitor::{DAConnectionMonitorSettings, MonitorEvent},
            policy::DAConnectionPolicy,
        },
        ConnectionMonitor, DAConnectionPolicySettings,
    },
    SubnetworkId,
};

// Metrics
const EVENT_SAMPLING: &str = "sampling";
const EVENT_VALIDATOR_DISPERSAL: &str = "validator_dispersal";
const EVENT_REPLICATION: &str = "replication";

pub struct ValidatorEventsStream {
    pub sampling_events_receiver: UnboundedReceiverStream<SamplingEvent>,
    pub validation_events_receiver: UnboundedReceiverStream<DaBlob>,
}

pub struct ValidatorSwarm<
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId> + Clone + 'static,
> {
    swarm: Swarm<
        ValidatorBehaviour<
            ConnectionBalancer<Membership>,
            ConnectionMonitor<Membership>,
            Membership,
        >,
    >,
    sampling_events_sender: UnboundedSender<SamplingEvent>,
    validation_events_sender: UnboundedSender<DaBlob>,
}

impl<Membership> ValidatorSwarm<Membership>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId> + Clone + Send,
{
    pub fn new(
        key: Keypair,
        membership: Membership,
        addresses: AddressBook,
        policy_settings: DAConnectionPolicySettings,
        monitor_settings: DAConnectionMonitorSettings,
        balancer_interval: Duration,
        redial_cooldown: Duration,
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

        (
            Self {
                swarm: Self::build_swarm(
                    key,
                    membership,
                    addresses,
                    balancer,
                    monitor,
                    redial_cooldown,
                ),
                sampling_events_sender,
                validation_events_sender,
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
        addresses: AddressBook,
        balancer: ConnectionBalancer<Membership>,
        monitor: ConnectionMonitor<Membership>,
        redial_cooldown: Duration,
    ) -> Swarm<
        ValidatorBehaviour<
            ConnectionBalancer<Membership>,
            ConnectionMonitor<Membership>,
            Membership,
        >,
    > {
        SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_quic()
            .with_behaviour(|key| {
                ValidatorBehaviour::new(
                    key,
                    membership,
                    addresses,
                    balancer,
                    monitor,
                    redial_cooldown,
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

    pub fn sample_request_channel(&mut self) -> UnboundedSender<(Membership::NetworkId, BlobId)> {
        self.swarm
            .behaviour()
            .sampling_behaviour()
            .sample_request_channel()
    }

    pub fn local_peer_id(&self) -> &PeerId {
        self.swarm.local_peer_id()
    }

    pub const fn protocol_swarm(
        &self,
    ) -> &Swarm<
        ValidatorBehaviour<
            ConnectionBalancer<Membership>,
            ConnectionMonitor<Membership>,
            Membership,
        >,
    > {
        &self.swarm
    }

    pub fn protocol_swarm_mut(
        &mut self,
    ) -> &mut Swarm<
        ValidatorBehaviour<
            ConnectionBalancer<Membership>,
            ConnectionMonitor<Membership>,
            Membership,
        >,
    > {
        &mut self.swarm
    }

    async fn handle_sampling_event(&mut self, event: SamplingEvent) {
        monitor_event(
            self.swarm.behaviour_mut().monitor_behaviour_mut(),
            MonitorEvent::from(&event),
        );
        handle_sampling_event(&self.sampling_events_sender, event).await;
    }

    async fn handle_dispersal_event(&mut self, event: DispersalEvent) {
        monitor_event(
            self.swarm.behaviour_mut().monitor_behaviour_mut(),
            MonitorEvent::from(&event),
        );
        handle_validator_dispersal_event(
            &self.validation_events_sender,
            self.swarm.behaviour_mut().replication_behaviour_mut(),
            event,
        )
        .await;
    }

    async fn handle_replication_event(&mut self, event: ReplicationEvent) {
        monitor_event(
            self.swarm.behaviour_mut().monitor_behaviour_mut(),
            MonitorEvent::from(&event),
        );
        handle_replication_event(&self.validation_events_sender, event).await;
    }

    async fn handle_behaviour_event(
        &mut self,
        event: ValidatorBehaviourEvent<
            ConnectionBalancer<Membership>,
            ConnectionMonitor<Membership>,
            Membership,
        >,
    ) {
        match event {
            ValidatorBehaviourEvent::Sampling(event) => {
                tracing::info!(
                    counter.behaviour_events_received = 1,
                    event = EVENT_SAMPLING
                );
                self.handle_sampling_event(event).await;
            }
            ValidatorBehaviourEvent::Dispersal(event) => {
                tracing::info!(
                    counter.behaviour_events_received = 1,
                    event = EVENT_VALIDATOR_DISPERSAL,
                    blob_size = event.blob_size()
                );
                self.handle_dispersal_event(event).await;
            }
            ValidatorBehaviourEvent::Replication(event) => {
                tracing::info!(
                    counter.behaviour_events_received = 1,
                    event = EVENT_REPLICATION,
                    blob_size = event.blob_size()
                );
                self.handle_replication_event(event).await;
            }
            _ => {}
        }
    }

    pub async fn run(mut self) {
        loop {
            if let Some(event) = self.swarm.next().await {
                debug!("Da swarm event received: {event:?}");
                match event {
                    SwarmEvent::Behaviour(behaviour_event) => {
                        self.handle_behaviour_event(behaviour_event).await;
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
        }
    }
}
