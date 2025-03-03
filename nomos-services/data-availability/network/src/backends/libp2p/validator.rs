use std::{fmt::Debug, marker::PhantomData, pin::Pin};

use futures::{
    future::{AbortHandle, Abortable, Aborted},
    Stream, StreamExt,
};
use kzgrs_backend::common::blob::DaBlob;
use libp2p::PeerId;
use nomos_core::da::BlobId;
use nomos_da_network_core::{swarm::validator::ValidatorSwarm, SubnetworkId};
use nomos_libp2p::ed25519;
use nomos_tracing::info_with_id;
use overwatch_rs::{overwatch::handle::OverwatchHandle, services::state::NoState};
use subnetworks_assignations::MembershipHandler;
use tokio::{
    sync::{broadcast, mpsc::UnboundedSender},
    task::JoinHandle,
};
use tokio_stream::wrappers::BroadcastStream;
use tracing::instrument;

use crate::backends::{
    libp2p::common::{
        dial_validator_subnetwork_peers, handle_sample_request, handle_validator_events_stream,
        DaNetworkBackendSettings, SamplingEvent, BROADCAST_CHANNEL_SIZE,
    },
    NetworkBackend,
};

/// Message that the backend replies to
#[derive(Debug)]
pub enum DaNetworkMessage {
    /// Kickstart a network sapling
    RequestSample {
        subnetwork_id: SubnetworkId,
        blob_id: BlobId,
    },
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
    Verifying(Box<DaBlob>),
}

/// DA network backend for validators
/// Internally uses a libp2p swarm composed of the [`ValidatorBehaviour`]
/// It forwards network messages to the corresponding subscription
/// channels/streams
pub struct DaNetworkValidatorBackend<Membership> {
    task: (AbortHandle, JoinHandle<Result<(), Aborted>>),
    replies_task: (AbortHandle, JoinHandle<Result<(), Aborted>>),
    sampling_request_channel: UnboundedSender<(SubnetworkId, BlobId)>,
    sampling_broadcast_receiver: broadcast::Receiver<SamplingEvent>,
    verifying_broadcast_receiver: broadcast::Receiver<DaBlob>,
    _membership: PhantomData<Membership>,
}

#[async_trait::async_trait]
impl<Membership> NetworkBackend for DaNetworkValidatorBackend<Membership>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
{
    type Settings = DaNetworkBackendSettings<Membership>;
    type State = NoState<Self::Settings>;
    type Message = DaNetworkMessage;
    type EventKind = DaNetworkEventKind;
    type NetworkEvent = DaNetworkEvent;

    fn new(config: Self::Settings, overwatch_handle: OverwatchHandle) -> Self {
        let keypair =
            libp2p::identity::Keypair::from(ed25519::Keypair::from(config.node_key.clone()));
        let (mut validator_swarm, validator_events_stream) = ValidatorSwarm::new(
            keypair,
            config.membership.clone(),
            config.addresses.clone().into_iter().collect(),
            config.policy_settings,
            config.monitor_settings,
            config.balancer_interval,
            config.redial_cooldown,
        );
        let address = config.listening_address;
        // put swarm to listen at the specified configuration address
        validator_swarm
            .protocol_swarm_mut()
            .listen_on(address.clone())
            .unwrap_or_else(|e| {
                panic!("Error listening on DA network with address {address}: {e}")
            });
        // Dial peers in the same subnetworks (Node might participate in multiple).
        let local_peer_id = *validator_swarm.local_peer_id();

        dial_validator_subnetwork_peers(
            &config.membership,
            &config.addresses,
            validator_swarm.protocol_swarm_mut(),
            local_peer_id,
        );

        let sampling_request_channel = validator_swarm.sample_request_channel();

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
            DaNetworkMessage::RequestSample {
                subnetwork_id,
                blob_id,
            } => {
                info_with_id!(&blob_id, "RequestSample");
                handle_sample_request(&self.sampling_request_channel, subnetwork_id, blob_id).await;
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
                    .map(|blob| Self::NetworkEvent::Verifying(Box::new(blob))),
            ),
        }
    }
}
