pub mod blend;
mod leadership;
mod messages;
pub mod network;
mod relays;
mod states;
pub mod storage;

use core::fmt::Debug;
use std::{collections::BTreeSet, fmt::Display, hash::Hash, path::PathBuf};

use cryptarchia_engine::{CryptarchiaState, Online, Slot};
use futures::StreamExt as _;
pub use leadership::LeaderConfig;
use network::NetworkAdapter;
use nomos_blend_service::BlendService;
use nomos_core::{
    block::{builder::BlockBuilder, Block},
    da::blob::{info::DispersedBlobInfo, metadata::Metadata as BlobMetadata, BlobSelect},
    header::{Builder, Header, HeaderId},
    proofs::leader_proof::Risc0LeaderProof,
    tx::{Transaction, TxSelect},
};
use nomos_da_sampling::{
    backend::DaSamplingServiceBackend, DaSamplingService, DaSamplingServiceMsg,
};
use nomos_ledger::{leader_proof::LeaderProof as _, LedgerState};
use nomos_mempool::{
    backend::RecoverableMempool, network::NetworkAdapter as MempoolAdapter, DaMempoolService,
    MempoolMsg, TxMempoolService,
};
use nomos_network::NetworkService;
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend, StorageService};
use nomos_time::{SlotTick, TimeService, TimeServiceMessage};
use overwatch::{
    services::{relay::OutboundRelay, AsServiceId, ServiceCore, ServiceData},
    DynError, OpaqueServiceResourcesHandle,
};
use rand::{RngCore, SeedableRng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_with::serde_as;
use services_utils::overwatch::{
    recovery::backends::FileBackendSettings, JsonFileBackend, RecoveryOperator,
};
use thiserror::Error;
use tokio::sync::{broadcast, oneshot, oneshot::Sender};
use tracing::{error, info, instrument, span, Level};
use tracing_futures::Instrument as _;

use crate::{
    leadership::Leader,
    relays::CryptarchiaConsensusRelays,
    states::{
        CryptarchiaConsensusState, CryptarchiaInitialisationStrategy, GenesisRecoveryStrategy,
        SecurityRecoveryStrategy,
    },
    storage::{adapters::StorageAdapter, StorageAdapter as _},
};

type MempoolRelay<Payload, Item, Key> = OutboundRelay<MempoolMsg<HeaderId, Payload, Item, Key>>;
type SamplingRelay<BlobId> = OutboundRelay<DaSamplingServiceMsg<BlobId>>;

// Limit the number of blocks returned by GetHeaders
const HEADERS_LIMIT: usize = 512;
const CRYPTARCHIA_ID: &str = "Cryptarchia";

#[derive(Debug, Clone, Error)]
pub enum Error {
    #[error("Ledger error: {0}")]
    Ledger(#[from] nomos_ledger::LedgerError<HeaderId>),
    #[error("Consensus error: {0}")]
    Consensus(#[from] cryptarchia_engine::Error<HeaderId>),
}

struct Cryptarchia<State> {
    ledger: nomos_ledger::Ledger<HeaderId>,
    consensus: cryptarchia_engine::Cryptarchia<HeaderId, State>,
}

impl<State: CryptarchiaState> Cryptarchia<State> {
    /// Initialize a new [`Cryptarchia`] instance.
    /// [`Cryptarchia`] must always be initialized from genesis.
    pub fn from_genesis(
        genesis_id: HeaderId,
        genesis_ledger_state: LedgerState,
        ledger_config: nomos_ledger::Config,
    ) -> Self {
        Self {
            consensus: <cryptarchia_engine::Cryptarchia<_, _>>::from_genesis(
                genesis_id,
                ledger_config.consensus_config,
            ),
            ledger: <nomos_ledger::Ledger<_>>::new(genesis_id, genesis_ledger_state, ledger_config),
        }
    }

    const fn tip(&self) -> HeaderId {
        self.consensus.tip()
    }

    fn tip_state(&self) -> &LedgerState {
        self.ledger
            .state(&self.tip())
            .expect("tip state not available")
    }

    const fn genesis(&self) -> HeaderId {
        self.consensus.genesis()
    }

    /// Create a new [`Cryptarchia`] instance with the updated state.
    /// This method adds a [`LedgerState`] and [`Header`] to the new instance
    /// without running validation.
    ///
    /// # Warning
    ///
    /// **This method bypasses safety checks** and can corrupt the state if used
    /// incorrectly.
    /// Only use for recovery, debugging, or other manipulations where the input
    /// is known to be valid.
    ///
    /// # Arguments
    ///
    /// * `header`: The header to apply.
    /// * `ledger_state`: The ledger state to apply.
    /// * `chain_length`: The position of the block in the chain.
    ///
    /// # Returns
    ///
    /// A new [`Cryptarchia`] instance with the updated state.
    pub fn apply_unchecked(
        &self,
        header: &Header,
        ledger_state: LedgerState,
        chain_length: u64,
    ) -> Self {
        let header_id = header.id();
        let ledger = self.ledger.apply_state_unchecked(header_id, ledger_state);
        let consensus = self.consensus.receive_block_unchecked(
            header_id,
            header.parent(),
            header.slot(),
            chain_length,
        );
        Self { ledger, consensus }
    }

    /// Create a new [`Cryptarchia`] with the updated state.
    #[must_use = "Returns a new instance with the updated state, without modifying the original."]
    fn try_apply_header(&self, header: &Header) -> Result<Self, Error> {
        let id = header.id();
        let parent = header.parent();
        let slot = header.slot();
        let ledger = self.ledger.try_update(
            id,
            parent,
            slot,
            header.leader_proof(),
            header.orphaned_proofs().iter().map(|imported_header| {
                (
                    imported_header.id(),
                    imported_header.leader_proof().to_orphan_proof(),
                )
            }),
        )?;
        let consensus = self.consensus.receive_block(id, parent, slot)?;

        Ok(Self { ledger, consensus })
    }

    fn epoch_state_for_slot(&self, slot: Slot) -> Option<&nomos_ledger::EpochState> {
        let tip = self.tip();
        let state = self.ledger.state(&tip).expect("no state for tip");
        let requested_epoch = self.ledger.config().epoch(slot);
        if state.epoch_state().epoch() == requested_epoch {
            Some(state.epoch_state())
        } else if requested_epoch == state.next_epoch_state().epoch() {
            Some(state.next_epoch_state())
        } else {
            None
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CryptarchiaSettings<Ts, Bs, NetworkAdapterSettings, BlendAdapterSettings> {
    #[serde(default)]
    pub transaction_selector_settings: Ts,
    #[serde(default)]
    pub blob_selector_settings: Bs,
    pub config: nomos_ledger::Config,
    pub genesis_state: LedgerState,
    pub leader_config: LeaderConfig,
    pub network_adapter_settings: NetworkAdapterSettings,
    pub blend_adapter_settings: BlendAdapterSettings,
    pub recovery_file: PathBuf,
}

impl<Ts, Bs, NetworkAdapterSettings, BlendAdapterSettings> FileBackendSettings
    for CryptarchiaSettings<Ts, Bs, NetworkAdapterSettings, BlendAdapterSettings>
{
    fn recovery_file(&self) -> &PathBuf {
        &self.recovery_file
    }
}

#[expect(clippy::allow_attributes_without_reason)]
pub struct CryptarchiaConsensus<
    NetAdapter,
    BlendAdapter,
    ClPool,
    ClPoolAdapter,
    DaPool,
    DaPoolAdapter,
    TxS,
    BS,
    Storage,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingRng,
    SamplingStorage,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
    TimeBackend,
    ApiAdapter,
    RuntimeServiceId,
> where
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Backend: 'static,
    NetAdapter::Settings: Send,
    BlendAdapter: blend::BlendAdapter<RuntimeServiceId>,
    BlendAdapter::Settings: Send,
    ClPool: RecoverableMempool<BlockId = HeaderId>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone,
    ClPool::Item: Clone + Eq + Hash + Debug + 'static,
    ClPool::Key: Debug + 'static,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    DaPool: RecoverableMempool<BlockId = HeaderId>,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Item: Clone + Eq + Hash + Debug + 'static,
    DaPool::Key: Debug + 'static,
    DaPool::Settings: Clone,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaPool::Item>,
    BS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = DaPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingRng: SeedableRng + RngCore,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId>,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    block_subscription_sender: broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    initial_state: <Self as ServiceData>::State,
}

impl<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    > ServiceData
    for CryptarchiaConsensus<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Settings: Send,
    BlendAdapter: blend::BlendAdapter<RuntimeServiceId>,
    BlendAdapter::Settings: Send,
    ClPool: RecoverableMempool<BlockId = HeaderId>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone,
    ClPool::Item: Clone + Eq + Hash + Debug,
    ClPool::Key: Debug,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    DaPool: RecoverableMempool<BlockId = HeaderId>,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Item: Clone + Eq + Hash + Debug,
    DaPool::Key: Debug,
    DaPool::Settings: Clone,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaPool::Item>,
    BS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = DaPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingRng: SeedableRng + RngCore,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId>,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
{
    type Settings = CryptarchiaSettings<
        TxS::Settings,
        BS::Settings,
        NetAdapter::Settings,
        BlendAdapter::Settings,
    >;
    type State = CryptarchiaConsensusState<
        TxS::Settings,
        BS::Settings,
        NetAdapter::Settings,
        BlendAdapter::Settings,
        TimeBackend::Settings,
    >;
    type StateOperator = RecoveryOperator<JsonFileBackend<Self::State, Self::Settings>>;
    type Message = ConsensusMsg<Block<ClPool::Item, DaPool::Item>>;
}

#[async_trait::async_trait]
impl<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    > ServiceCore<RuntimeServiceId>
    for CryptarchiaConsensus<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    NetAdapter: NetworkAdapter<RuntimeServiceId, Tx = ClPool::Item, BlobCertificate = DaPool::Item>
        + Clone
        + Send
        + Sync
        + 'static,
    NetAdapter::Settings: Send + Sync + 'static,
    BlendAdapter: blend::BlendAdapter<RuntimeServiceId, Tx = ClPool::Item, BlobCertificate = DaPool::Item>
        + Clone
        + Send
        + Sync
        + 'static,
    BlendAdapter::Settings: Send + Sync + 'static,
    ClPool: RecoverableMempool<BlockId = HeaderId> + Send + Sync + 'static,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone + Send + Sync + 'static,
    ClPool::Item: Transaction<Hash = ClPool::Key>
        + Debug
        + Clone
        + Eq
        + Hash
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    ClPool::Key: Debug + Send + Sync,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>
        + Send
        + Sync
        + 'static,
    DaPool: RecoverableMempool<BlockId = HeaderId, Key = SamplingBackend::BlobId>
        + Send
        + Sync
        + 'static,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Settings: Clone + Send + Sync + 'static,
    // TODO: Change to specific certificate bounds here
    DaPool::Item: DispersedBlobInfo<BlobId = DaPool::Key>
        + BlobMetadata
        + Debug
        + Clone
        + Eq
        + Hash
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key> + Send + Sync + 'static,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    TxS: TxSelect<Tx = ClPool::Item> + Clone + Send + Sync + 'static,
    TxS::Settings: Send + Sync + 'static,
    BS: BlobSelect<BlobId = DaPool::Item> + Clone + Send + Sync + 'static,
    BS::Settings: Send + Sync + 'static,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item, DaPool::Item>> + TryInto<Block<ClPool::Item, DaPool::Item>>,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + Send + 'static,
    SamplingBackend::BlobId: Debug + Ord + Send + Sync + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingRng: SeedableRng + RngCore,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<Self>
        + AsServiceId<NetworkService<NetAdapter::Backend, RuntimeServiceId>>
        + AsServiceId<BlendService<BlendAdapter::Backend, BlendAdapter::Network, RuntimeServiceId>>
        + AsServiceId<TxMempoolService<ClPoolAdapter, ClPool, RuntimeServiceId>>
        + AsServiceId<
            DaMempoolService<
                DaPoolAdapter,
                DaPool,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingRng,
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
                ApiAdapter,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<
            DaSamplingService<
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingRng,
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
                ApiAdapter,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<StorageService<Storage, RuntimeServiceId>>
        + AsServiceId<TimeService<TimeBackend, RuntimeServiceId>>,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        initial_state: Self::State,
    ) -> Result<Self, DynError> {
        let (block_subscription_sender, _) = broadcast::channel(16);

        Ok(Self {
            service_resources_handle,
            block_subscription_sender,
            initial_state,
        })
    }

    #[expect(clippy::too_many_lines, reason = "TODO: Address this at some point.")]
    async fn run(mut self) -> Result<(), DynError> {
        let relays: CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        > = CryptarchiaConsensusRelays::from_service_resources_handle::<_, _, _, _, _, _>(
            &self.service_resources_handle,
        )
        .await;

        let CryptarchiaSettings {
            config: ledger_config,
            genesis_state,
            transaction_selector_settings,
            blob_selector_settings,
            leader_config,
            network_adapter_settings,
            blend_adapter_settings,
            ..
        } = self
            .service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let genesis_id = HeaderId::from([0; 32]);

        let (mut cryptarchia, mut leader) = Self::build_cryptarchia(
            self.initial_state,
            genesis_id,
            genesis_state,
            ledger_config,
            leader_config,
            &relays,
            &mut self.block_subscription_sender,
        )
        .await;

        let network_adapter =
            NetAdapter::new(network_adapter_settings, relays.network_relay().clone()).await;
        let tx_selector = TxS::new(transaction_selector_settings);
        let blob_selector = BS::new(blob_selector_settings);

        let mut incoming_blocks = network_adapter.blocks_stream().await?;

        let mut slot_timer = {
            let (sender, receiver) = oneshot::channel();
            relays
                .time_relay()
                .send(TimeServiceMessage::Subscribe { sender })
                .await
                .expect("Request time subscription to time service should succeed");
            receiver.await?
        };

        let blend_adapter =
            BlendAdapter::new(blend_adapter_settings, relays.blend_relay().clone()).await;

        async {
            loop {
                tokio::select! {
                    Some(block) = incoming_blocks.next() => {
                        Self::log_received_block(&block);
                        cryptarchia = Self::process_block(
                            cryptarchia,
                            &mut leader,
                            block,
                            &relays,
                            &mut self.block_subscription_sender,
                        )
                        .await;

                        self.service_resources_handle.state_updater.update(Some(Self::State::from_cryptarchia(&cryptarchia, &leader)));

                        tracing::info!(counter.consensus_processed_blocks = 1);
                    }

                    Some(SlotTick { slot, .. }) = slot_timer.next() => {
                        let parent = cryptarchia.tip();
                        let note_tree = cryptarchia.tip_state().lead_commitments();
                        tracing::debug!("ticking for slot {}", u64::from(slot));

                        let Some(epoch_state) = cryptarchia.epoch_state_for_slot(slot) else {
                            tracing::error!("trying to propose a block for slot {} but epoch state is not available", u64::from(slot));
                            continue;
                        };
                        if let Some(proof) = leader.build_proof_for(note_tree, epoch_state, slot, parent).await {
                            tracing::debug!("proposing block...");
                            // TODO: spawn as a separate task?
                            let block = Self::propose_block(
                                parent,
                                slot,
                                proof,
                                tx_selector.clone(),
                                blob_selector.clone(),
                                &relays
                            ).await;

                            if let Some(block) = block {
                                // apply our own block
                                cryptarchia = Self::process_block(
                                    cryptarchia,
                                    &mut leader,
                                    block.clone(),
                                    &relays,
                                    &mut self.block_subscription_sender
                                ).await;
                                blend_adapter.blend(block).await;
                            }
                        }
                    }

                    Some(msg) = self.service_resources_handle.inbound_relay.next() => {
                        Self::process_message(&cryptarchia, &self.block_subscription_sender, msg);
                    }
                }
            }
            // it sucks to use "Cryptarchia" when we have the Self::SERVICE_ID.
            // Somehow it just do not let refer to the type to reference it.
            // Probably related to too many generics.
        }.instrument(span!(Level::TRACE, CRYPTARCHIA_ID)).await;
        Ok(())
    }
}

impl<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    >
    CryptarchiaConsensus<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    NetAdapter: NetworkAdapter<RuntimeServiceId> + Clone + Send + Sync + 'static,
    NetAdapter::Settings: Send,
    BlendAdapter: blend::BlendAdapter<RuntimeServiceId> + Clone + Send + Sync + 'static,
    BlendAdapter::Settings: Send,
    ClPool: RecoverableMempool<BlockId = HeaderId> + Send + Sync + 'static,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone + Send + Sync + 'static,
    ClPool::Item: Transaction<Hash = ClPool::Key>
        + Debug
        + Clone
        + Eq
        + Hash
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    ClPool::Key: Debug + Send + Sync,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>
        + Send
        + Sync
        + 'static,
    DaPool::Item: DispersedBlobInfo<BlobId = DaPool::Key>
        + BlobMetadata
        + Debug
        + Clone
        + Eq
        + Hash
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    DaPool: RecoverableMempool<BlockId = HeaderId, Key = SamplingBackend::BlobId>
        + Send
        + Sync
        + 'static,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Settings: Clone + Send + Sync + 'static,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key> + Send + Sync + 'static,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    TxS: TxSelect<Tx = ClPool::Item> + Clone + Send + Sync + 'static,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaPool::Item> + Clone + Send + Sync + 'static,
    BS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item, DaPool::Item>> + TryInto<Block<ClPool::Item, DaPool::Item>>,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + Ord + Send + Sync + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingRng: SeedableRng + RngCore,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
{
    fn process_message<State: CryptarchiaState>(
        cryptarchia: &Cryptarchia<State>,
        block_channel: &broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
        msg: ConsensusMsg<Block<ClPool::Item, DaPool::Item>>,
    ) {
        match msg {
            ConsensusMsg::Info { tx } => {
                let info = CryptarchiaInfo {
                    tip: cryptarchia.tip(),
                    slot: cryptarchia
                        .ledger
                        .state(&cryptarchia.tip())
                        .expect("tip state not available")
                        .slot(),
                    height: cryptarchia
                        .consensus
                        .branches()
                        .get(&cryptarchia.tip())
                        .expect("tip branch not available")
                        .length(),
                };
                tx.send(info).unwrap_or_else(|e| {
                    tracing::error!("Could not send consensus info through channel: {:?}", e);
                });
            }
            ConsensusMsg::BlockSubscribe { sender } => {
                sender.send(block_channel.subscribe()).unwrap_or_else(|_| {
                    tracing::error!("Could not subscribe to block subscription channel");
                });
            }
            ConsensusMsg::GetHeaders { from, to, tx } => {
                // default to tip block if not present
                let from = from.unwrap_or_else(|| cryptarchia.tip());
                // default to genesis block if not present
                let to = to.unwrap_or_else(|| cryptarchia.genesis());

                let mut res = Vec::new();
                let mut cur = from;

                let branches = cryptarchia.consensus.branches();
                while let Some(h) = branches.get(&cur) {
                    res.push(h.id());
                    // limit the response size
                    if cur == to || cur == cryptarchia.genesis() || res.len() >= HEADERS_LIMIT {
                        break;
                    }
                    cur = h.parent();
                }

                tx.send(res)
                    .unwrap_or_else(|_| tracing::error!("could not send blocks through channel"));
            }
        }
    }

    #[expect(clippy::allow_attributes_without_reason)]
    #[expect(clippy::type_complexity)]
    async fn validate_received_block(
        block: &Block<ClPool::Item, DaPool::Item>,
        relays: &CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
    ) -> bool {
        let sampled_blobs = match get_sampled_blobs(relays.sampling_relay().clone()).await {
            Ok(sampled_blobs) => sampled_blobs,
            Err(error) => {
                error!("Unable to retrieved sampled blobs: {error}");
                return false;
            }
        };
        if !Self::validate_blocks_blobs(block, &sampled_blobs) {
            error!("Invalid block: {block:?}");
            return false;
        }
        true
    }

    /// Add a [`Block`] to [`Cryptarchia`].
    /// A [`Block`] is only added if it's valid
    /// ([`CryptarchiaConsensus::validate_received_block`]).
    #[expect(clippy::allow_attributes_without_reason)]
    #[expect(clippy::type_complexity)]
    #[instrument(level = "debug", skip(cryptarchia, leader, relays))]
    async fn process_block<State: CryptarchiaState>(
        cryptarchia: Cryptarchia<State>,
        leader: &mut leadership::Leader,
        block: Block<ClPool::Item, DaPool::Item>,
        relays: &CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
        block_broadcaster: &mut broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    ) -> Cryptarchia<State> {
        tracing::debug!("received proposal {:?}", block);
        if !Self::validate_received_block(&block, relays).await {
            return cryptarchia;
        }
        Self::process_block_unchecked(cryptarchia, leader, block, relays, block_broadcaster).await
    }

    /// Add a [`Block`] to [`Cryptarchia`].
    /// This method does not validate the block.
    #[expect(clippy::allow_attributes_without_reason)]
    #[expect(clippy::type_complexity)]
    #[instrument(level = "debug", skip(cryptarchia, leader, relays))]
    async fn process_block_unchecked<State: CryptarchiaState>(
        mut cryptarchia: Cryptarchia<State>,
        leader: &mut leadership::Leader,
        block: Block<ClPool::Item, DaPool::Item>,
        relays: &CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
        block_broadcaster: &mut broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    ) -> Cryptarchia<State> {
        // TODO: filter on time?
        let header = block.header();
        let id = header.id();

        match cryptarchia.try_apply_header(header) {
            Ok(new_state) => {
                // update leader
                leader.follow_chain(header.parent(), id, header.leader_proof().nullifier());

                // remove included content from mempool
                mark_in_block(
                    relays.cl_mempool_relay().clone(),
                    block.transactions().map(Transaction::hash),
                    id,
                )
                .await;
                mark_in_block(
                    relays.da_mempool_relay().clone(),
                    block.blobs().map(DispersedBlobInfo::blob_id),
                    id,
                )
                .await;

                mark_blob_in_block(
                    relays.sampling_relay().clone(),
                    block.blobs().map(DispersedBlobInfo::blob_id).collect(),
                )
                .await;

                if let Err(e) = relays
                    .storage_adapter()
                    .store_block(header.id(), block.clone())
                    .await
                {
                    error!("Could not store block {e}");
                }

                if let Err(e) = block_broadcaster.send(block) {
                    tracing::error!("Could not notify block to services {e}");
                }

                cryptarchia = new_state;
            }
            Err(
                Error::Ledger(nomos_ledger::LedgerError::ParentNotFound(parent))
                | Error::Consensus(cryptarchia_engine::Error::ParentMissing(parent)),
            ) => {
                tracing::debug!("missing parent {:?}", parent);
                // TODO: request parent block
            }
            Err(e) => tracing::debug!("invalid block {:?}: {e:?}", block),
        }

        cryptarchia
    }

    #[expect(clippy::allow_attributes_without_reason)]
    #[expect(clippy::type_complexity)]
    #[instrument(level = "debug", skip(tx_selector, blob_selector, relays))]
    async fn propose_block(
        parent: HeaderId,
        slot: Slot,
        proof: Risc0LeaderProof,
        tx_selector: TxS,
        blob_selector: BS,
        relays: &CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
    ) -> Option<Block<ClPool::Item, DaPool::Item>> {
        let mut output = None;
        let cl_txs = get_mempool_contents(relays.cl_mempool_relay().clone());
        let da_certs = get_mempool_contents(relays.da_mempool_relay().clone());
        let blobs_ids = get_sampled_blobs(relays.sampling_relay().clone());
        match futures::join!(cl_txs, da_certs, blobs_ids) {
            (Ok(cl_txs), Ok(da_blobs_info), Ok(blobs_ids)) => {
                let block = BlockBuilder::new(
                    tx_selector,
                    blob_selector,
                    Builder::new(parent, slot, proof),
                )
                .with_transactions(cl_txs)
                .with_blobs_info(
                    da_blobs_info.filter(move |info| blobs_ids.contains(&info.blob_id())),
                )
                .build()
                .expect("Proposal block should always succeed to be built");
                tracing::debug!("proposed block with id {:?}", block.header().id());
                output = Some(block);
            }
            (tx_error, da_certificate_error, blobs_error) => {
                if let Err(_tx_error) = tx_error {
                    tracing::error!("Could not fetch block cl transactions");
                }
                if let Err(_da_certificate_error) = da_certificate_error {
                    tracing::error!("Could not fetch block da certificates");
                }
                if let Err(_blobs_error) = blobs_error {
                    tracing::error!("Could not fetch block da blobs");
                }
            }
        }
        output
    }

    fn validate_blocks_blobs(
        block: &Block<ClPool::Item, DaPool::Item>,
        sampled_blobs_ids: &BTreeSet<DaPool::Key>,
    ) -> bool {
        let validated_blobs = block
            .blobs()
            .all(|blob| sampled_blobs_ids.contains(&blob.blob_id()));
        validated_blobs
    }

    fn log_received_block(block: &Block<ClPool::Item, DaPool::Item>) {
        let content_size = block.header().content_size();
        let transactions = block.cl_transactions_len();
        let blobs = block.bl_blobs_len();

        tracing::info!(
            counter.received_blocks = 1,
            transactions = transactions,
            blobs = blobs,
            bytes = content_size
        );
        tracing::info!(
            histogram.received_blocks_data = content_size,
            transactions = transactions,
            blobs = blobs
        );
    }

    /// Retrieves the blocks in the range from `from` to `to` from the storage.
    /// Both `from` and `to` are included in the range.
    /// This is implemented here, and not as a method of `StorageAdapter`, to
    /// simplify the panic and error message handling.
    ///
    /// # Panics
    ///
    /// Panics if any of the blocks in the range are not found in the storage.
    ///
    /// # Parameters
    ///
    /// * `from` - The header id of the first block in the range. Must be a
    ///   valid header.
    /// * `to` - The header id of the last block in the range. Must be a valid
    ///   header.
    ///
    /// # Returns
    ///
    /// A vector of blocks in the range from `from` to `to`.
    /// If no blocks are found, returns an empty vector.
    /// If any of the [`HeaderId`]s are invalid, returns an error with the first
    /// invalid header id.
    async fn get_blocks_in_range(
        from: HeaderId,
        to: HeaderId,
        storage_adapter: &StorageAdapter<Storage, TxS::Tx, BS::BlobId, RuntimeServiceId>,
    ) -> Vec<Block<ClPool::Item, DaPool::Item>> {
        // Due to the blocks traversal order, this yields `to..from` order
        let blocks = futures::stream::unfold(to, |header_id| async move {
            if header_id == from {
                None
            } else {
                let block = storage_adapter
                    .get_block(&header_id)
                    .await
                    .unwrap_or_else(|| {
                        panic!("Could not retrieve block {to} from storage during recovery")
                    });
                let parent_header_id = block.header().parent();
                Some((block, parent_header_id))
            }
        });

        // To avoid confusion, the order is reversed so it fits the natural `from..to`
        // order
        blocks.collect::<Vec<_>>().await.into_iter().rev().collect()
    }

    /// Builds cryptarchia
    /// The build process is determined by the initial state passed:
    /// - If the initial state doesn't contain any recovery information,
    ///   cryptarchia is built from genesis.
    /// - If it does, the recovery process is started: Recovery is done from
    ///   genesis or security depending on the available information.
    ///
    /// # Arguments
    ///
    /// * `initial_state` - The initial state of cryptarchia.
    /// * `genesis_id` - The genesis block id.
    /// * `genesis_state` - The genesis ledger state.
    /// * `leader` - The leader instance. It needs to be a Leader initialised to
    ///   genesis. This function will update the leader if needed.
    /// * `ledger_config` - The ledger configuration.
    /// * `relays` - The relays object containing all the necessary relays for
    ///   the consensus.
    /// * `block_subscription_sender` - The broadcast channel to send the blocks
    ///   to the services.
    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusState and CryptarchiaConsensusRelays amount of generics."
    )]
    async fn build_cryptarchia(
        mut initial_state: CryptarchiaConsensusState<
            TxS::Settings,
            BS::Settings,
            NetAdapter::Settings,
            BlendAdapter::Settings,
            TimeBackend::Settings,
        >,
        genesis_id: HeaderId,
        genesis_state: LedgerState,
        ledger_config: nomos_ledger::Config,
        leader_config: LeaderConfig,
        relays: &CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
        block_subscription_sender: &mut broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    ) -> (Cryptarchia<Online>, Leader) {
        match initial_state.recovery_strategy() {
            CryptarchiaInitialisationStrategy::Genesis => {
                info!("Building Cryptarchia from genesis.");
                Self::build_from_genesis(genesis_id, genesis_state, leader_config, ledger_config)
            }
            CryptarchiaInitialisationStrategy::RecoveryFromGenesis(strategy) => {
                info!("Recovering Cryptarchia with Genesis strategy.");
                Self::recover_from_genesis(
                    strategy,
                    genesis_id,
                    genesis_state,
                    leader_config,
                    ledger_config,
                    relays,
                    block_subscription_sender,
                )
                .await
            }
            CryptarchiaInitialisationStrategy::RecoveryFromSecurity(strategy) => {
                info!("Recovering Cryptarchia with Security strategy.");
                Self::recover_from_security(
                    *strategy,
                    genesis_id,
                    genesis_state,
                    leader_config,
                    ledger_config,
                    relays,
                    block_subscription_sender,
                )
                .await
            }
        }
    }

    fn build_from_genesis(
        genesis_id: HeaderId,
        genesis_state: LedgerState,
        leader_config: LeaderConfig,
        ledger_config: nomos_ledger::Config,
    ) -> (Cryptarchia<Online>, Leader) {
        let leader = Leader::from_genesis(genesis_id, leader_config, ledger_config);
        let cryptarchia = Cryptarchia::from_genesis(genesis_id, genesis_state, ledger_config);

        (cryptarchia, leader)
    }

    /// Recovers cryptarchia from genesis to the saved tip
    ///
    /// # Arguments
    ///
    /// * `GenesisRecoveryStrategy` - Strategy instance containing the genesis
    ///   recovery information.
    /// * `genesis_id` - The genesis block id.
    /// * `genesis_state` - The genesis ledger state.
    /// * `leader_config` - The leader configuration.
    /// * `ledger_config` - The ledger configuration.
    /// * `relays` - The relays object containing all the necessary relays for
    ///   the consensus.
    /// * `block_subscription_sender` - The broadcast channel to send the blocks
    ///   to the services.
    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusRelays amount of generics."
    )]
    async fn recover_from_genesis(
        GenesisRecoveryStrategy { tip }: GenesisRecoveryStrategy,
        genesis_id: HeaderId,
        genesis_state: LedgerState,
        leader_config: LeaderConfig,
        ledger_config: nomos_ledger::Config,
        relays: &CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
        block_subscription_sender: &mut broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    ) -> (Cryptarchia<Online>, Leader) {
        let mut cryptarchia =
            <Cryptarchia<Online>>::from_genesis(genesis_id, genesis_state, ledger_config);

        let mut leader = Leader::from_genesis(genesis_id, leader_config, ledger_config);

        let blocks = Self::get_blocks_in_range(genesis_id, tip, relays.storage_adapter()).await;

        // Skip genesis blocks since Cryptarchia is already built from it
        let blocks = blocks.into_iter().skip(1);

        for block in blocks {
            cryptarchia = Self::process_block(
                cryptarchia,
                &mut leader,
                block,
                relays,
                block_subscription_sender,
            )
            .await;
        }

        (cryptarchia, leader)
    }

    /// Recovers cryptarchia from a previously saved security block to the saved
    /// tip
    ///
    /// # Arguments
    ///
    /// * `SecurityRecoveryStrategy` - Strategy instance containing the security
    ///   recovery information.
    /// * `genesis_id` - The genesis block id.
    /// * `genesis_state` - The genesis ledger state.
    /// * `leader_config` - The leader configuration.
    /// * `ledger_config` - The ledger configuration.
    /// * `relays` - The relays object containing all the necessary relays for
    ///   the consensus.
    /// * `block_subscription_sender` - The broadcast channel to send the blocks
    ///   to the services.
    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusRelays amount of generics."
    )]
    async fn recover_from_security(
        SecurityRecoveryStrategy {
            tip,
            security_block_id,
            security_ledger_state,
            security_leader_notes,
            security_block_chain_length,
        }: SecurityRecoveryStrategy,
        genesis_id: HeaderId,
        genesis_state: LedgerState,
        leader_config: LeaderConfig,
        ledger_config: nomos_ledger::Config,
        relays: &CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
        block_subscription_sender: &mut broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    ) -> (Cryptarchia<Online>, Leader) {
        let mut cryptarchia =
            <Cryptarchia<Online>>::from_genesis(genesis_id, genesis_state, ledger_config);
        let mut leader = Leader::new(
            security_block_id,
            security_leader_notes,
            leader_config.nf_sk,
            ledger_config,
        );

        let blocks =
            Self::get_blocks_in_range(security_block_id, tip, relays.storage_adapter()).await;

        // Apply security block with _unchecked_ since it's already validated and in its
        // final form.
        let mut blocks_iter = blocks.into_iter();
        let security_block = blocks_iter
            .next()
            .expect("Security block must be present when recovering from security block.");
        cryptarchia = cryptarchia.apply_unchecked(
            security_block.header(),
            security_ledger_state,
            security_block_chain_length,
        );

        // Apply remaining blocks with _unchecked_ to bypass blobs validation
        for block in blocks_iter {
            cryptarchia = Self::process_block_unchecked(
                cryptarchia,
                &mut leader,
                block,
                relays,
                block_subscription_sender,
            )
            .await;
        }

        (cryptarchia, leader)
    }
}

#[derive(Debug)]
pub enum ConsensusMsg<Block> {
    Info {
        tx: Sender<CryptarchiaInfo>,
    },
    BlockSubscribe {
        sender: oneshot::Sender<broadcast::Receiver<Block>>,
    },
    GetHeaders {
        from: Option<HeaderId>,
        to: Option<HeaderId>,
        tx: Sender<Vec<HeaderId>>,
    },
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptarchiaInfo {
    pub tip: HeaderId,
    pub slot: Slot,
    pub height: u64,
}

async fn get_mempool_contents<Payload, Item, Key>(
    mempool: OutboundRelay<MempoolMsg<HeaderId, Payload, Item, Key>>,
) -> Result<Box<dyn Iterator<Item = Item> + Send>, tokio::sync::oneshot::error::RecvError>
where
    Key: Send,
    Payload: Send,
{
    let (reply_channel, rx) = tokio::sync::oneshot::channel();

    mempool
        .send(MempoolMsg::View {
            ancestor_hint: [0; 32].into(),
            reply_channel,
        })
        .await
        .unwrap_or_else(|(e, _)| eprintln!("Could not get transactions from mempool {e}"));

    rx.await
}

async fn mark_in_block<Payload, Item, Key>(
    mempool: OutboundRelay<MempoolMsg<HeaderId, Payload, Item, Key>>,
    ids: impl Iterator<Item = Key>,
    block: HeaderId,
) where
    Key: Send,
    Payload: Send,
{
    mempool
        .send(MempoolMsg::MarkInBlock {
            ids: ids.collect(),
            block,
        })
        .await
        .unwrap_or_else(|(e, _)| tracing::error!("Could not mark items in block: {e}"));
}

async fn mark_blob_in_block<BlobId: Debug + Send>(
    sampling_relay: SamplingRelay<BlobId>,
    blobs_id: Vec<BlobId>,
) {
    if let Err((_e, DaSamplingServiceMsg::MarkInBlock { blobs_id })) = sampling_relay
        .send(DaSamplingServiceMsg::MarkInBlock { blobs_id })
        .await
    {
        error!("Error marking in block for blobs ids: {blobs_id:?}");
    }
}

async fn get_sampled_blobs<BlobId>(
    sampling_relay: SamplingRelay<BlobId>,
) -> Result<BTreeSet<BlobId>, DynError>
where
    BlobId: Send,
{
    let (sender, receiver) = oneshot::channel();
    sampling_relay
        .send(DaSamplingServiceMsg::GetValidatedBlobs {
            reply_channel: sender,
        })
        .await
        .map_err(|(error, _)| Box::new(error) as DynError)?;
    receiver.await.map_err(|error| Box::new(error) as DynError)
}
