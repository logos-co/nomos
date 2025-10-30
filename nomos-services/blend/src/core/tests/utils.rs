use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use libp2p::Multiaddr;
use nomos_blend_message::crypto::keys::Ed25519PrivateKey;
use nomos_blend_scheduling::{
    EncapsulatedMessage,
    membership::{Membership, Node},
    message_blend::crypto::IncomingEncapsulatedMessageWithValidatedPublicHeader,
};
use nomos_network::{NetworkService, backends::NetworkBackend};
use overwatch::{
    overwatch::OverwatchHandle,
    services::{ServiceData, relay::OutboundRelay},
};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};

use crate::core::{
    backends::{BlendBackend, EpochInfo, PublicInfo, SessionInfo},
    network::NetworkAdapter,
    settings::BlendConfig,
};

const CHANNEL_SIZE: usize = 10;

pub fn new_stream<Item>() -> (impl Stream<Item = Item> + Unpin, mpsc::Sender<Item>) {
    let (sender, receiver) = mpsc::channel(CHANNEL_SIZE);
    (ReceiverStream::new(receiver), sender)
}

pub struct TestBlendBackend;

#[async_trait]
impl<NodeId, Rng, ProofsVerifier, RuntimeServiceId>
    BlendBackend<NodeId, Rng, ProofsVerifier, RuntimeServiceId> for TestBlendBackend
where
    NodeId: Send + 'static,
{
    type Settings = ();

    fn new(
        _service_config: BlendConfig<Self::Settings>,
        _overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        _current_public_info: PublicInfo<NodeId>,
        _rng: Rng,
    ) -> Self {
        Self
    }

    fn shutdown(self) {}
    async fn publish(&self, _msg: EncapsulatedMessage) {}
    async fn rotate_session(&mut self, _new_session_info: SessionInfo<NodeId>) {}
    async fn complete_session_transition(&mut self) {}
    async fn rotate_epoch(&mut self, _new_epoch_public_info: EpochInfo) {}
    async fn complete_epoch_transition(&mut self) {}

    fn listen_to_incoming_messages(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = IncomingEncapsulatedMessageWithValidatedPublicHeader> + Send>>
    {
        unimplemented!()
    }
}

pub struct TestNetworkAdapter;

#[async_trait]
impl<RuntimeServiceId> NetworkAdapter<RuntimeServiceId> for TestNetworkAdapter {
    type Backend = TestNetworkBackend;
    type BroadcastSettings = ();

    fn new(
        _network_relay: OutboundRelay<
            <NetworkService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self {
        Self
    }

    async fn broadcast(&self, _message: Vec<u8>, _broadcast_settings: Self::BroadcastSettings) {}
}

pub struct TestNetworkBackend {
    pubsub_sender: broadcast::Sender<()>,
    chainsync_sender: broadcast::Sender<()>,
}

#[async_trait]
impl<RuntimeServiceId> NetworkBackend<RuntimeServiceId> for TestNetworkBackend {
    type Settings = ();
    type Message = ();
    type PubSubEvent = ();
    type ChainSyncEvent = ();

    fn new(_config: Self::Settings, _overwatch_handle: OverwatchHandle<RuntimeServiceId>) -> Self {
        let (pubsub_sender, _) = broadcast::channel(CHANNEL_SIZE);
        let (chainsync_sender, _) = broadcast::channel(CHANNEL_SIZE);
        Self {
            pubsub_sender,
            chainsync_sender,
        }
    }

    async fn process(&self, _msg: Self::Message) {}

    async fn subscribe_to_pubsub(&mut self) -> BroadcastStream<Self::PubSubEvent> {
        BroadcastStream::new(self.pubsub_sender.subscribe())
    }

    async fn subscribe_to_chainsync(&mut self) -> BroadcastStream<Self::ChainSyncEvent> {
        BroadcastStream::new(self.chainsync_sender.subscribe())
    }
}

pub type NodeId = u8;

pub fn new_membership(size: u8) -> (Membership<NodeId>, Ed25519PrivateKey) {
    let nodes = (0..size)
        .map(|id| Node {
            id,
            address: Multiaddr::empty(),
            public_key: private_key(id).public_key(),
        })
        .collect::<Vec<_>>();
    let local_private_key = private_key(nodes.first().unwrap().id);
    (
        Membership::new(&nodes, &local_private_key.public_key()),
        local_private_key,
    )
}

fn private_key(id: NodeId) -> Ed25519PrivateKey {
    Ed25519PrivateKey::from([id; 32])
}
