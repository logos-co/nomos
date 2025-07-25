use std::{fmt::Debug, marker::PhantomData, pin::Pin};

use futures::{
    future::Aborted,
    stream::{AbortHandle, Abortable},
    Stream, StreamExt as _,
};
use kzgrs_backend::common::share::DaShare;
use libp2p::PeerId;
use log::error;
use nomos_core::da::BlobId;
use nomos_da_network_core::{
    maintenance::{balancer::ConnectionBalancerCommand, monitor::ConnectionMonitorCommand},
    protocols::dispersal::executor::behaviour::DispersalExecutorEvent,
    swarm::{executor::ExecutorSwarm, validator::SwarmSettings, BalancerStats, MonitorStats},
    SubnetworkId,
};
use nomos_libp2p::ed25519;
use nomos_tracing::info_with_id;
use overwatch::{overwatch::handle::OverwatchHandle, services::state::NoState};
use serde::{Deserialize, Serialize};
use subnetworks_assignations::MembershipHandler;
use tokio::{
    sync::{broadcast, mpsc::UnboundedSender, oneshot},
    task::JoinHandle,
    time,
};
use tokio_stream::wrappers::{BroadcastStream, IntervalStream, UnboundedReceiverStream};
use tracing::instrument;

use super::common::CommitmentsEvent;
use crate::{
    backends::{
        libp2p::common::{
            handle_balancer_command, handle_commitments_request, handle_monitor_command,
            handle_sample_request, handle_validator_events_stream, DaNetworkBackendSettings,
            SamplingEvent, BROADCAST_CHANNEL_SIZE,
        },
        NetworkBackend,
    },
    membership::handler::DaMembershipHandler,
    DaAddressbook,
};

/// Message that the backend replies to
#[derive(Debug)]
pub enum ExecutorDaNetworkMessage<BalancerStats, MonitorStats> {
    /// Kickstart a network sapling
    RequestSample {
        blob_id: BlobId,
    },
    RequestCommitments {
        blob_id: BlobId,
    },
    RequestDispersal {
        subnetwork_id: SubnetworkId,
        da_share: Box<DaShare>,
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
    Commitments,
    Verifying,
    Dispersal,
}

/// DA network incoming events
#[derive(Debug)]
pub enum DaNetworkEvent {
    Sampling(SamplingEvent),
    Commitments(CommitmentsEvent),
    Verifying(Box<DaShare>),
    Dispersal(DispersalExecutorEvent),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DaNetworkExecutorBackendSettings {
    pub validator_settings: DaNetworkBackendSettings,
    pub num_subnets: u16,
}

/// DA network backend for validators
/// Internally uses a libp2p swarm composed of the [`ExecutorBehaviour`]
/// It forwards network messages to the corresponding subscription
/// channels/streams
pub struct DaNetworkExecutorBackend<Membership>
where
    Membership: MembershipHandler,
{
    task: (AbortHandle, JoinHandle<Result<(), Aborted>>),
    verifier_replies_task: (AbortHandle, JoinHandle<Result<(), Aborted>>),
    executor_replies_task: (AbortHandle, JoinHandle<Result<(), Aborted>>),
    shares_request_channel: UnboundedSender<BlobId>,
    commitments_request_channel: UnboundedSender<BlobId>,
    sampling_broadcast_receiver: broadcast::Receiver<SamplingEvent>,
    commitments_broadcast_receiver: broadcast::Receiver<CommitmentsEvent>,
    verifying_broadcast_receiver: broadcast::Receiver<DaShare>,
    dispersal_broadcast_receiver: broadcast::Receiver<DispersalExecutorEvent>,
    dispersal_shares_sender: UnboundedSender<(Membership::NetworkId, DaShare)>,
    balancer_command_sender: UnboundedSender<ConnectionBalancerCommand<BalancerStats>>,
    monitor_command_sender: UnboundedSender<ConnectionMonitorCommand<MonitorStats>>,
    _membership: PhantomData<Membership>,
}

#[async_trait::async_trait]
impl<Membership, RuntimeServiceId> NetworkBackend<RuntimeServiceId>
    for DaNetworkExecutorBackend<Membership>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    BalancerStats: Debug + Serialize + Send + Sync + 'static,
{
    type Settings = DaNetworkExecutorBackendSettings;
    type State = NoState<Self::Settings>;
    type Message = ExecutorDaNetworkMessage<BalancerStats, MonitorStats>;
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
        let subnet_refresh_signal = Box::pin(
            IntervalStream::new(time::interval(config.validator_settings.refresh_interval))
                .map(|_| ()),
        );

        let keypair = libp2p::identity::Keypair::from(ed25519::Keypair::from(
            config.validator_settings.node_key.clone(),
        ));
        let (mut executor_swarm, executor_events_stream) = ExecutorSwarm::new(
            keypair,
            membership,
            addressbook,
            SwarmSettings {
                policy_settings: config.validator_settings.policy_settings,
                monitor_settings: config.validator_settings.monitor_settings,
                balancer_interval: config.validator_settings.balancer_interval,
                redial_cooldown: config.validator_settings.redial_cooldown,
                replication_settings: config.validator_settings.replication_settings,
                subnets_settings: config.validator_settings.subnets_settings,
            },
            subnet_refresh_signal,
        );
        let address = config.validator_settings.listening_address;
        // put swarm to listen at the specified configuration address
        executor_swarm
            .protocol_swarm_mut()
            .listen_on(address.clone())
            .unwrap_or_else(|e| {
                panic!("Error listening on DA network with address {address}: {e}")
            });

        let shares_request_channel = executor_swarm.shares_request_channel();
        let commitments_request_channel = executor_swarm.commitments_request_channel();
        let dispersal_shares_sender = executor_swarm.dispersal_shares_channel();
        let balancer_command_sender = executor_swarm.balancer_command_channel();
        let monitor_command_sender = executor_swarm.monitor_command_channel();

        let (task_abort_handle, abort_registration) = AbortHandle::new_pair();
        let task = (
            task_abort_handle,
            overwatch_handle
                .runtime()
                .spawn(Abortable::new(executor_swarm.run(), abort_registration)),
        );

        let (sampling_broadcast_sender, sampling_broadcast_receiver) =
            broadcast::channel(BROADCAST_CHANNEL_SIZE);
        let (commitments_broadcast_sender, commitments_broadcast_receiver) =
            broadcast::channel(BROADCAST_CHANNEL_SIZE);
        let (verifying_broadcast_sender, verifying_broadcast_receiver) =
            broadcast::channel(BROADCAST_CHANNEL_SIZE);
        let (dispersal_broadcast_sender, dispersal_broadcast_receiver) =
            broadcast::channel(BROADCAST_CHANNEL_SIZE);
        let (verifier_replies_task_abort_handle, verifier_replies_task_abort_registration) =
            AbortHandle::new_pair();
        let verifier_replies_task = (
            verifier_replies_task_abort_handle,
            overwatch_handle.runtime().spawn(Abortable::new(
                handle_validator_events_stream(
                    executor_events_stream.validator_events_stream,
                    sampling_broadcast_sender,
                    commitments_broadcast_sender,
                    verifying_broadcast_sender,
                ),
                verifier_replies_task_abort_registration,
            )),
        );
        let (executor_replies_task_abort_handle, executor_replies_task_abort_registration) =
            AbortHandle::new_pair();

        let executor_replies_task = (
            executor_replies_task_abort_handle,
            overwatch_handle.runtime().spawn(Abortable::new(
                handle_executor_dispersal_events_stream(
                    executor_events_stream.dispersal_events_receiver,
                    dispersal_broadcast_sender,
                ),
                executor_replies_task_abort_registration,
            )),
        );

        Self {
            task,
            verifier_replies_task,
            executor_replies_task,
            shares_request_channel,
            commitments_request_channel,
            sampling_broadcast_receiver,
            commitments_broadcast_receiver,
            verifying_broadcast_receiver,
            dispersal_broadcast_receiver,
            dispersal_shares_sender,
            balancer_command_sender,
            monitor_command_sender,
            _membership: PhantomData,
        }
    }

    fn shutdown(&mut self) {
        let Self {
            task: (task_handle, _),
            verifier_replies_task: (verifier_handle, _),
            executor_replies_task: (executor_handle, _),
            ..
        } = self;
        task_handle.abort();
        verifier_handle.abort();
        executor_handle.abort();
    }

    #[instrument(skip_all)]
    async fn process(&self, msg: Self::Message) {
        match msg {
            ExecutorDaNetworkMessage::RequestSample { blob_id } => {
                info_with_id!(&blob_id, "RequestSample");
                handle_sample_request(&self.shares_request_channel, blob_id).await;
            }
            ExecutorDaNetworkMessage::RequestCommitments { blob_id } => {
                info_with_id!(&blob_id, "RequestSample");
                handle_commitments_request(&self.commitments_request_channel, blob_id).await;
            }
            ExecutorDaNetworkMessage::RequestDispersal {
                subnetwork_id,
                da_share,
            } => {
                info_with_id!(&da_share.blob_id(), "RequestDispersal");
                if let Err(e) = self
                    .dispersal_shares_sender
                    .send((subnetwork_id, *da_share))
                {
                    error!("Could not send internal blob to underlying dispersal behaviour: {e}");
                }
            }
            ExecutorDaNetworkMessage::MonitorRequest(command) => {
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
            ExecutorDaNetworkMessage::BalancerStats(response_sender) => {
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
            DaNetworkEventKind::Commitments => Box::pin(
                BroadcastStream::new(self.commitments_broadcast_receiver.resubscribe())
                    .filter_map(|event| async { event.ok() })
                    .map(Self::NetworkEvent::Commitments),
            ),
            DaNetworkEventKind::Verifying => Box::pin(
                BroadcastStream::new(self.verifying_broadcast_receiver.resubscribe())
                    .filter_map(|event| async { event.ok() })
                    .map(|share| Self::NetworkEvent::Verifying(Box::new(share))),
            ),
            DaNetworkEventKind::Dispersal => Box::pin(
                BroadcastStream::new(self.dispersal_broadcast_receiver.resubscribe())
                    .filter_map(|event| async { event.ok() })
                    .map(Self::NetworkEvent::Dispersal),
            ),
        }
    }
}

async fn handle_executor_dispersal_events_stream(
    mut dispersal_events_receiver: UnboundedReceiverStream<DispersalExecutorEvent>,
    dispersal_broadcast_sender: broadcast::Sender<DispersalExecutorEvent>,
) {
    while let Some(event) = dispersal_events_receiver.next().await {
        if let Err(e) = dispersal_broadcast_sender.send(event) {
            error!("Error forwarding internal dispersal executor event: {e}");
        }
    }
}
