use crate::backends::libp2p::common::{
    handle_sample_request, handle_validator_events_stream, SamplingEvent,
};
use crate::backends::NetworkBackend;
use futures::{Stream, StreamExt};
use kzgrs_backend::common::blob::DaBlob;
use libp2p::identity::ed25519;
use libp2p::{Multiaddr, PeerId};
use nomos_core::da::BlobId;
use nomos_da_network_core::swarm::validator::ValidatorSwarm;
use nomos_da_network_core::SubnetworkId;
use nomos_libp2p::secret_key_serde;
use overwatch_rs::overwatch::handle::OverwatchHandle;
use overwatch_rs::services::state::NoState;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::pin::Pin;
use subnetworks_assignations::MembershipHandler;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::BroadcastStream;

const BROADCAST_CHANNEL_SIZE: usize = 128;

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
pub struct DaNetworkExecutorBackend<Membership> {
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DaNetworkExecutorBackendSettings<Membership> {
    // Identification Secp256k1 private key in Hex format (`0x123...abc`). Default random.
    #[serde(with = "secret_key_serde", default = "ed25519::SecretKey::generate")]
    pub node_key: ed25519::SecretKey,
    /// Membership of DA network PoV set
    pub membership: Membership,
    pub addresses: Vec<(PeerId, Multiaddr)>,
    pub listening_address: Multiaddr,
}

#[async_trait::async_trait]
impl<Membership> NetworkBackend for DaNetworkExecutorBackend<Membership>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
{
    type Settings = DaNetworkExecutorBackendSettings<Membership>;
    type State = NoState<Self::Settings>;
    type Message = DaNetworkMessage;
    type EventKind = DaNetworkEventKind;
    type NetworkEvent = DaNetworkEvent;

    fn new(config: Self::Settings, overwatch_handle: OverwatchHandle) -> Self {
        let keypair =
            libp2p::identity::Keypair::from(ed25519::Keypair::from(config.node_key.clone()));
        let (mut validator_swarm, events_streams) = ValidatorSwarm::new(
            keypair,
            config.membership.clone(),
            config.addresses.clone().into_iter().collect(),
        );
        let sampling_request_channel = validator_swarm
            .protocol_swarm()
            .behaviour()
            .sampling_behaviour()
            .sample_request_channel();
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
        let mut connected_peers = HashSet::new();

        config
            .membership
            .membership(&local_peer_id)
            .iter()
            .flat_map(|subnet| config.membership.members_of(subnet))
            .filter(|peer| peer != &local_peer_id)
            .filter_map(|peer| {
                config
                    .addresses
                    .iter()
                    .find(|(p, _)| p == &peer)
                    .map(|(_, addr)| (peer, addr.clone()))
            })
            .for_each(|(peer, addr)| {
                // Only dial if we haven't already connected to this peer.
                if connected_peers.insert(peer) {
                    validator_swarm
                        .dial(addr)
                        .expect("Node should be able to dial peer in a subnet");
                }
            });

        let task = overwatch_handle.runtime().spawn(validator_swarm.run());
        let (sampling_broadcast_sender, sampling_broadcast_receiver) =
            broadcast::channel(BROADCAST_CHANNEL_SIZE);
        let (verifying_broadcast_sender, verifying_broadcast_receiver) =
            broadcast::channel(BROADCAST_CHANNEL_SIZE);
        let replies_task = overwatch_handle
            .runtime()
            .spawn(handle_validator_events_stream(
                events_streams,
                sampling_broadcast_sender,
                verifying_broadcast_sender,
            ));

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
