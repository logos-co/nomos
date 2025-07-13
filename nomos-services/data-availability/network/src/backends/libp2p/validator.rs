use std::{fmt::Debug, marker::PhantomData, pin::Pin};

use futures::{
    future::{AbortHandle, Abortable, Aborted},
    Stream, StreamExt as _,
};
use kzgrs_backend::common::share::DaShare;
use libp2p::PeerId;
use nomos_core::da::BlobId;
use nomos_da_network_core::{
    maintenance::{balancer::ConnectionBalancerCommand, monitor::ConnectionMonitorCommand},
    swarm::{
        validator::{SwarmSettings, ValidatorSwarm},
        BalancerStats, MonitorStats,
    },
    SubnetworkId,
};
use nomos_libp2p::ed25519;
use nomos_tracing::info_with_id;
use overwatch::{overwatch::handle::OverwatchHandle, services::state::NoState};
use serde::Serialize;
use subnetworks_assignations::MembershipHandler;
use tokio::{
    sync::{broadcast, mpsc::UnboundedSender, oneshot},
    task::JoinHandle,
    time,
};
use tokio_stream::wrappers::{BroadcastStream, IntervalStream};
use tracing::instrument;

use crate::{
    backends::{
        libp2p::common::{
            handle_balancer_command, handle_monitor_command, handle_sample_request,
            handle_validator_events_stream, DaNetworkBackendSettings, SamplingEvent,
            BROADCAST_CHANNEL_SIZE,
        },
        NetworkBackend,
    },
    membership::handler::DaMembershipHandler,
    DaAddressbook,
};

/// Message that the backend replies to
#[derive(Debug)]
pub enum DaNetworkMessage<BalancerStats, MonitorStats>
where
    BalancerStats: Debug + Serialize,
{
    /// Kickstart a network sapling
    RequestSample {
        blob_id: BlobId,
    },
    MonitorRequest(ConnectionMonitorCommand<MonitorStats>),
    BalancerStats(oneshot::Sender<BalancerStats>),
}

/// Events types to subscribe to
/// * Sampling: Incoming sampling events [success/fail]
/// * Incoming blobs to be verified
#[derive(Debug)]
pub enum DaNetworkEventKind {
    Sampling,
    Verifying,
}

/// DA network incoming events
#[derive(Debug)]
pub enum DaNetworkEvent {
    Sampling(SamplingEvent),
    Verifying(Box<DaShare>),
}

/// DA network backend for validators
/// Internally uses a libp2p swarm composed of the [`ValidatorBehaviour`]
/// It forwards network messages to the corresponding subscription
/// channels/streams
pub struct DaNetworkValidatorBackend<Membership> {
    task: (AbortHandle, JoinHandle<Result<(), Aborted>>),
    replies_task: (AbortHandle, JoinHandle<Result<(), Aborted>>),
    sampling_request_channel: UnboundedSender<BlobId>,
    balancer_command_sender: UnboundedSender<ConnectionBalancerCommand<BalancerStats>>,
    monitor_command_sender: UnboundedSender<ConnectionMonitorCommand<MonitorStats>>,
    sampling_broadcast_receiver: broadcast::Receiver<SamplingEvent>,
    verifying_broadcast_receiver: broadcast::Receiver<DaShare>,
    _membership: PhantomData<Membership>,
}

#[async_trait::async_trait]
impl<Membership, RuntimeServiceId> NetworkBackend<RuntimeServiceId>
    for DaNetworkValidatorBackend<Membership>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    BalancerStats: Debug + Serialize + Send + Sync + 'static,
{
    type Settings = DaNetworkBackendSettings;
    type State = NoState<Self::Settings>;
    type Message = DaNetworkMessage<BalancerStats, MonitorStats>;
    type EventKind = DaNetworkEventKind;
    type NetworkEvent = DaNetworkEvent;
    type Membership = DaMembershipHandler<Membership>;
    type Addressbook = DaAddressbook;

    fn new(
        config: Self::Settings,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        membership: Self::Membership,
        addressbook: Self::Addressbook,
    ) -> Self {
        // TODO: If there is no requirement to subscribe to block number events in chain
        // service, and an approximate duration is enough for sampling to hold
        // temporal connections - remove this message.
        let subnet_refresh_signal =
            Box::pin(IntervalStream::new(time::interval(config.refresh_interval)).map(|_| ()));

        let keypair =
            libp2p::identity::Keypair::from(ed25519::Keypair::from(config.node_key.clone()));
        let (mut validator_swarm, validator_events_stream) = ValidatorSwarm::new(
            keypair,
            membership,
            addressbook,
            SwarmSettings {
                policy_settings: config.policy_settings,
                monitor_settings: config.monitor_settings,
                balancer_interval: config.balancer_interval,
                redial_cooldown: config.redial_cooldown,
                replication_settings: config.replication_settings,
                subnets_settings: config.subnets_settings,
            },
            subnet_refresh_signal,
        );
        let address = config.listening_address;
        // put swarm to listen at the specified configuration address
        validator_swarm
            .protocol_swarm_mut()
            .listen_on(address.clone())
            .unwrap_or_else(|e| {
                panic!("Error listening on DA network with address {address}: {e}")
            });

        let sampling_request_channel = validator_swarm.sample_request_channel();
        let balancer_command_sender = validator_swarm.balancer_command_channel();
        let monitor_command_sender = validator_swarm.monitor_command_channel();

        let (task_abort_handle, abort_registration) = AbortHandle::new_pair();
        let task = (
            task_abort_handle,
            overwatch_handle
                .runtime()
                .spawn(Abortable::new(validator_swarm.run(), abort_registration)),
        );
        let (sampling_broadcast_sender, sampling_broadcast_receiver) =
            broadcast::channel(BROADCAST_CHANNEL_SIZE);
        let (verifying_broadcast_sender, verifying_broadcast_receiver) =
            broadcast::channel(BROADCAST_CHANNEL_SIZE);
        let (replies_task_abort_handle, replies_task_abort_registration) = AbortHandle::new_pair();
        let replies_task = (
            replies_task_abort_handle,
            overwatch_handle.runtime().spawn(Abortable::new(
                handle_validator_events_stream(
                    validator_events_stream,
                    sampling_broadcast_sender,
                    verifying_broadcast_sender,
                ),
                replies_task_abort_registration,
            )),
        );

        Self {
            task,
            replies_task,
            sampling_request_channel,
            balancer_command_sender,
            monitor_command_sender,
            sampling_broadcast_receiver,
            verifying_broadcast_receiver,
            _membership: PhantomData,
        }
    }

    fn shutdown(&mut self) {
        let Self {
            task: (task_handle, _),
            replies_task: (replies_handle, _),
            ..
        } = self;
        task_handle.abort();
        replies_handle.abort();
    }

    #[instrument(skip_all)]
    async fn process(&self, msg: Self::Message) {
        match msg {
            DaNetworkMessage::RequestSample { blob_id } => {
                info_with_id!(&blob_id, "RequestSample");
                handle_sample_request(&self.sampling_request_channel, blob_id).await;
            }
            DaNetworkMessage::MonitorRequest(command) => {
                match command.peer_id() {
                    Some(peer_id) => {
                        tracing::info!(%peer_id, "{}", command.discriminant());
                    }
                    None => {
                        tracing::info!("{}", command.discriminant());
                    }
                }
                handle_monitor_command(&self.monitor_command_sender, command).await;
            }
            DaNetworkMessage::BalancerStats(response_sender) => {
                tracing::info!("BalancerStats");
                handle_balancer_command(&self.balancer_command_sender, response_sender).await;
            }
        }
    }

    async fn subscribe(
        &mut self,
        event: Self::EventKind,
    ) -> Pin<Box<dyn Stream<Item = Self::NetworkEvent> + Send>> {
        match event {
            DaNetworkEventKind::Sampling => Box::pin(
                BroadcastStream::new(self.sampling_broadcast_receiver.resubscribe())
                    .filter_map(|event| async { event.ok() })
                    .map(Self::NetworkEvent::Sampling),
            ),
            DaNetworkEventKind::Verifying => Box::pin(
                BroadcastStream::new(self.verifying_broadcast_receiver.resubscribe())
                    .filter_map(|event| async { event.ok() })
                    .map(|share| Self::NetworkEvent::Verifying(Box::new(share))),
            ),
        }
    }
}
