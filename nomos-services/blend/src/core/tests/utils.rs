use std::{num::NonZeroU64, pin::Pin, sync::Arc};

use async_trait::async_trait;
use futures::Stream;
use nomos_blend_message::crypto::keys::Ed25519PrivateKey;
use nomos_blend_scheduling::{
    EncapsulatedMessage,
    membership::Membership,
    message_blend::crypto::{
        IncomingEncapsulatedMessageWithValidatedPublicHeader, SessionCryptographicProcessorSettings,
    },
};
use nomos_core::mantle::keys::SecretKey;
use nomos_network::{NetworkService, backends::NetworkBackend};
use overwatch::{
    overwatch::{OverwatchHandle, commands::OverwatchCommand},
    services::{ServiceData, relay::OutboundRelay, state::StateUpdater},
};
use tempfile::NamedTempFile;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};

use crate::{
    core::{
        backends::{BlendBackend, EpochInfo, PublicInfo, SessionInfo},
        network::NetworkAdapter,
        settings::{
            BlendConfig, CoverTrafficSettings, MessageDelayerSettings, SchedulerSettings,
            ZkSettings,
        },
        state::RecoveryServiceState,
    },
    settings::TimingSettings,
    test_utils,
};

pub type NodeId = [u8; 32];

/// Creates a membership with the given size and returns it along with the
/// private key of the local node.
pub fn new_membership(size: u8) -> (Membership<NodeId>, Ed25519PrivateKey) {
    let ids = (0..size).map(|i| [i; 32]).collect::<Vec<_>>();
    let local_id = *ids.first().unwrap();
    (
        test_utils::membership::membership(&ids, local_id),
        test_utils::membership::key(local_id).0,
    )
}

pub fn settings<BackendSettings>(
    local_private_key: Ed25519PrivateKey,
    minimum_network_size: NonZeroU64,
    backend_settings: BackendSettings,
    timing_settings: TimingSettings,
) -> (BlendConfig<BackendSettings>, NamedTempFile) {
    let recovery_file = NamedTempFile::new().unwrap();
    let settings = BlendConfig {
        backend: backend_settings,
        crypto: SessionCryptographicProcessorSettings {
            non_ephemeral_signing_key: local_private_key,
            num_blend_layers: 1,
        },
        scheduler: SchedulerSettings {
            cover: CoverTrafficSettings {
                message_frequency_per_round: 1.0.try_into().unwrap(),
                redundancy_parameter: 0,
                intervals_for_safety_buffer: 0,
            },
            delayer: MessageDelayerSettings {
                maximum_release_delay_in_rounds: 1.try_into().unwrap(),
            },
        },
        time: timing_settings,
        zk: ZkSettings {
            sk: SecretKey::zero(),
        },
        minimum_network_size,
        recovery_path: recovery_file.path().to_path_buf(),
    };
    (settings, recovery_file)
}

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

#[expect(clippy::type_complexity, reason = "a test utility")]
pub fn dummy_overwatch_resources<BackendSettings, BroadcastSettings, RuntimeServiceId>() -> (
    OverwatchHandle<RuntimeServiceId>,
    mpsc::Receiver<OverwatchCommand<RuntimeServiceId>>,
    StateUpdater<Option<RecoveryServiceState<BackendSettings, BroadcastSettings>>>,
    watch::Receiver<Option<RecoveryServiceState<BackendSettings, BroadcastSettings>>>,
) {
    let (cmd_sender, cmd_receiver) = mpsc::channel(CHANNEL_SIZE);
    let handle =
        OverwatchHandle::<RuntimeServiceId>::new(tokio::runtime::Handle::current(), cmd_sender);
    let (state_sender, state_receiver) = watch::channel(None);
    let state_updater = StateUpdater::<
        Option<RecoveryServiceState<BackendSettings, BroadcastSettings>>,
    >::new(Arc::new(state_sender));

    (handle, cmd_receiver, state_updater, state_receiver)
}
