use crate::backends::libp2p::common::{
    handle_sample_request, initialize_backend, DaNetworkBackendSettings, InitializedBackendData,
    SamplingEvent,
};
use crate::backends::NetworkBackend;
use futures::{Stream, StreamExt};
use kzgrs_backend::common::blob::DaBlob;
use libp2p::PeerId;
use nomos_core::da::BlobId;
use nomos_da_network_core::SubnetworkId;
use overwatch_rs::overwatch::handle::OverwatchHandle;
use overwatch_rs::services::state::NoState;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::pin::Pin;
use subnetworks_assignations::MembershipHandler;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::BroadcastStream;

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
/// It forwards network messages to the corresponding subscription channels/streams
pub struct DaNetworkValidatorBackend<Membership> {
    // TODO: this join handles should be cancelable tasks. We should add an stop method for
    // the `NetworkBackend` trait so if the service is stopped the backend can gracefully handle open
    // sub-tasks as well.
    #[allow(dead_code)]
    task: JoinHandle<()>,
    #[allow(dead_code)]
    replies_task: JoinHandle<()>,
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
        let InitializedBackendData {
            task,
            replies_task,
            sampling_request_channel,
            sampling_broadcast_receiver,
            verifying_broadcast_receiver,
        } = initialize_backend(config, overwatch_handle);

        Self {
            task,
            replies_task,
            sampling_request_channel,
            sampling_broadcast_receiver,
            verifying_broadcast_receiver,
            _membership: Default::default(),
        }
    }

    async fn process(&self, msg: Self::Message) {
        match msg {
            DaNetworkMessage::RequestSample {
                subnetwork_id,
                blob_id,
            } => {
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
