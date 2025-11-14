use std::{num::NonZeroU64, pin::Pin, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::Stream;
use groth16::Field as _;
use nomos_blend_message::{
    crypto::{
        keys::{Ed25519PrivateKey, Ed25519PublicKey},
        proofs::{
            PoQVerificationInputsMinusSigningKey,
            quota::{
                ProofOfQuota,
                inputs::prove::{
                    private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs},
                    public::{CoreInputs, LeaderInputs},
                },
            },
            selection::{ProofOfSelection, inputs::VerifyInputs},
        },
    },
    encap::ProofsVerifier,
    reward::{self, BlendingTokenCollector},
};
use nomos_blend_scheduling::{
    EncapsulatedMessage,
    membership::Membership,
    message_blend::{
        crypto::{
            IncomingEncapsulatedMessageWithValidatedPublicHeader,
            SessionCryptographicProcessorSettings,
        },
        provers::{
            BlendLayerProof, ProofsGeneratorSettings, core_and_leader::CoreAndLeaderProofsGenerator,
        },
    },
    message_scheduler::{
        self, MessageScheduler, ProcessedMessageScheduler,
        session_info::SessionInfo as SchedulerSessionInfo,
    },
};
use nomos_core::{crypto::ZkHash, sdp::SessionNumber};
use nomos_network::{NetworkService, backends::NetworkBackend};
use nomos_utils::math::NonNegativeF64;
use overwatch::{
    overwatch::{OverwatchHandle, commands::OverwatchCommand},
    services::{ServiceData, relay::OutboundRelay, state::StateUpdater},
};
use poq::CORE_MERKLE_TREE_HEIGHT;
use tempfile::NamedTempFile;
use tokio::sync::{
    broadcast::{self},
    mpsc, watch,
};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use zksign::SecretKey;

use crate::{
    core::{
        backends::{BlendBackend, EpochInfo, PublicInfo, SessionInfo},
        network::NetworkAdapter,
        processor::CoreCryptographicProcessor,
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

/// Creates a [`BlendConfig`] with the given parameters and reasonable defaults
/// for the rest.
///
/// Also returns a [`NamedTempFile`] used for service recovery
/// that must not be dropped, as doing so will delete the underlying temp file.
pub fn settings<BackendSettings>(
    local_private_key: Ed25519PrivateKey,
    minimum_network_size: NonZeroU64,
    backend_settings: BackendSettings,
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
                intervals_for_safety_buffer: 0,
            },
            delayer: MessageDelayerSettings {
                maximum_release_delay_in_rounds: 1.try_into().unwrap(),
            },
        },
        time: TimingSettings {
            rounds_per_session: 10.try_into().unwrap(),
            rounds_per_interval: 5.try_into().unwrap(),
            round_duration: Duration::from_secs(1),
            rounds_per_observation_window: 5.try_into().unwrap(),
            rounds_per_session_transition_period: 2.try_into().unwrap(),
            epoch_transition_period_in_slots: 1.try_into().unwrap(),
        },
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

pub struct TestBlendBackend {
    // To notify tests about events occurring within the backend.
    event_sender: broadcast::Sender<TestBlendBackendEvent>,
}

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
        let (event_sender, _) = broadcast::channel(CHANNEL_SIZE);
        Self { event_sender }
    }

    fn shutdown(self) {}
    async fn publish(&self, _msg: EncapsulatedMessage) {}
    async fn rotate_session(&mut self, _new_session_info: SessionInfo<NodeId>) {}

    async fn complete_session_transition(&mut self) {
        // Notify tests that the backend completed the session transition.
        self.event_sender
            .send(TestBlendBackendEvent::SessionTransitionCompleted)
            .unwrap();
    }

    async fn rotate_epoch(&mut self, _new_epoch_public_info: EpochInfo) {}
    async fn complete_epoch_transition(&mut self) {}

    fn listen_to_incoming_messages(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = IncomingEncapsulatedMessageWithValidatedPublicHeader> + Send>>
    {
        unimplemented!()
    }
}

impl TestBlendBackend {
    /// Subscribes to backend test events.
    pub fn subscribe_to_events(&self) -> broadcast::Receiver<TestBlendBackendEvent> {
        self.event_sender.subscribe()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestBlendBackendEvent {
    SessionTransitionCompleted,
}

/// Waits for the given event to be received on the provided channel.
/// All other events are ignored.
///
/// It panics if the channel is lagged or closed.
pub async fn wait_for_blend_backend_event(
    receiver: &mut broadcast::Receiver<TestBlendBackendEvent>,
    event: TestBlendBackendEvent,
) {
    loop {
        let received_event = receiver
            .recv()
            .await
            .expect("channel shouldn't be closed or lagged");
        if received_event == event {
            return;
        }
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

pub fn new_crypto_processor(
    settings: &SessionCryptographicProcessorSettings,
    public_info: &PublicInfo<NodeId>,
    poq_core_quota_inputs: ProofOfCoreQuotaInputs,
) -> CoreCryptographicProcessor<NodeId, MockCoreAndLeaderProofsGenerator, MockProofsVerifier> {
    let minimum_network_size = u64::try_from(public_info.session.membership.size())
        .expect("membership size must fit into u64")
        .try_into()
        .expect("minimum_network_size must be non-zero");
    CoreCryptographicProcessor::try_new_with_core_condition_check(
        public_info.session.membership.clone(),
        minimum_network_size,
        settings,
        PoQVerificationInputsMinusSigningKey {
            session: public_info.session.session_number,
            core: public_info.session.core_public_inputs,
            leader: public_info.epoch,
        },
        poq_core_quota_inputs,
    )
    .expect("crypto processor must be created successfully")
}

pub fn new_public_info(session: u64, membership: Membership<NodeId>) -> PublicInfo<NodeId> {
    PublicInfo {
        session: SessionInfo {
            session_number: session,
            membership,
            core_public_inputs: CoreInputs {
                zk_root: ZkHash::ZERO,
                quota: 10,
            },
        },
        epoch: LeaderInputs {
            pol_ledger_aged: ZkHash::ZERO,
            pol_epoch_nonce: ZkHash::ZERO,
            message_quota: 10,
            total_stake: 10,
        },
    }
}

pub fn new_poq_core_quota_inputs() -> ProofOfCoreQuotaInputs {
    ProofOfCoreQuotaInputs {
        core_sk: ZkHash::ZERO,
        core_path_and_selectors: [(ZkHash::ZERO, false); CORE_MERKLE_TREE_HEIGHT],
    }
}

pub fn new_blending_token_collector(
    public_info: &PublicInfo<NodeId>,
    message_frequency_per_round: NonNegativeF64,
) -> BlendingTokenCollector {
    BlendingTokenCollector::new(
        &reward::SessionInfo::new(
            public_info.session.session_number,
            &public_info.epoch.pol_epoch_nonce,
            public_info
                .session
                .membership
                .size()
                .try_into()
                .expect("num_core_nodes must fit into u64"),
            public_info.session.core_public_inputs.quota,
            message_frequency_per_round,
        )
        .expect("session info must be created successfully"),
    )
}

pub struct MockProcessedMessageScheduler<ProcessedMessage> {
    messages: Vec<ProcessedMessage>,
}

impl<ProcessedMessage> Default for MockProcessedMessageScheduler<ProcessedMessage> {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
        }
    }
}

impl<Rng, ProcessedMessage> MessageScheduler<Rng, ProcessedMessage, EncapsulatedMessage>
    for MockProcessedMessageScheduler<ProcessedMessage>
{
    fn new(_: SchedulerSessionInfo, _: Rng, _: message_scheduler::Settings) -> Self {
        Self::default()
    }

    fn queue_data_message(&mut self, _: EncapsulatedMessage) {
        unimplemented!()
    }
}

impl<ProcessedMessage> ProcessedMessageScheduler<ProcessedMessage>
    for MockProcessedMessageScheduler<ProcessedMessage>
{
    fn schedule_processed_message(&mut self, message: ProcessedMessage) {
        self.messages.push(message);
    }
}

impl<ProcessedMessage> MockProcessedMessageScheduler<ProcessedMessage> {
    pub fn num_scheduled_messages(&self) -> usize {
        self.messages.len()
    }
}

pub struct MockCoreAndLeaderProofsGenerator(SessionNumber);

#[async_trait]
impl CoreAndLeaderProofsGenerator for MockCoreAndLeaderProofsGenerator {
    fn new(settings: ProofsGeneratorSettings, _: ProofOfCoreQuotaInputs) -> Self {
        Self(settings.public_inputs.session)
    }

    fn rotate_epoch(&mut self, _: LeaderInputs) {}
    fn set_epoch_private(&mut self, _: ProofOfLeadershipQuotaInputs) {}

    async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof> {
        Some(session_based_dummy_proofs(self.0))
    }

    async fn get_next_leader_proof(&mut self) -> Option<BlendLayerProof> {
        Some(session_based_dummy_proofs(self.0))
    }
}

#[derive(Debug, Clone)]
pub struct MockProofsVerifier(SessionNumber);

impl ProofsVerifier for MockProofsVerifier {
    type Error = ();

    fn new(public_inputs: PoQVerificationInputsMinusSigningKey) -> Self {
        Self(public_inputs.session)
    }

    fn start_epoch_transition(&mut self, _new_pol_inputs: LeaderInputs) {}

    fn complete_epoch_transition(&mut self) {}

    fn verify_proof_of_quota(
        &self,
        proof: ProofOfQuota,
        _signing_key: &Ed25519PublicKey,
    ) -> Result<ZkHash, Self::Error> {
        let expected_proof = session_based_dummy_proofs(self.0).proof_of_quota;
        if proof == expected_proof {
            Ok(ZkHash::ZERO)
        } else {
            Err(())
        }
    }

    fn verify_proof_of_selection(
        &self,
        proof: ProofOfSelection,
        _inputs: &VerifyInputs,
    ) -> Result<(), Self::Error> {
        let expected_proof = session_based_dummy_proofs(self.0).proof_of_selection;
        if proof == expected_proof {
            Ok(())
        } else {
            Err(())
        }
    }
}

fn session_based_dummy_proofs(session: SessionNumber) -> BlendLayerProof {
    let session_bytes = session.to_le_bytes();
    BlendLayerProof {
        proof_of_quota: ProofOfQuota::from_bytes_unchecked({
            let mut bytes = [0u8; _];
            bytes[..session_bytes.len()].copy_from_slice(&session_bytes);
            bytes
        }),
        proof_of_selection: ProofOfSelection::from_bytes_unchecked({
            let mut bytes = [0u8; _];
            bytes[..session_bytes.len()].copy_from_slice(&session_bytes);
            bytes
        }),
        ephemeral_signing_key: Ed25519PrivateKey::generate(),
    }
}

impl MockProofsVerifier {
    pub fn session_number(&self) -> SessionNumber {
        self.0
    }
}
