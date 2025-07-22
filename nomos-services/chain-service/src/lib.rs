pub mod blend;
mod leadership;
mod messages;
pub mod network;
mod processor;
mod relays;
mod states;
pub mod storage;
mod sync;

use core::fmt::Debug;
use std::{
    collections::{BTreeSet, HashSet},
    fmt::Display,
    path::PathBuf,
    time::Duration,
};

use cryptarchia_engine::{CryptarchiaState, Online, PrunedBlocks, Slot};
use cryptarchia_sync::GetTipResponse;
use futures::StreamExt as _;
pub use leadership::LeaderConfig;
use network::NetworkAdapter;
use nomos_blend_service::BlendService;
use nomos_core::{
    block::{builder::BlockBuilder, Block},
    da::blob::{info::DispersedBlobInfo, metadata::Metadata as BlobMetadata, BlobSelect},
    header::{Builder, Header, HeaderId},
    mantle::{Transaction, TxSelect},
    proofs::leader_proof::Risc0LeaderProof,
};
use nomos_da_sampling::{
    backend::DaSamplingServiceBackend, DaSamplingService, DaSamplingServiceMsg,
};
use nomos_ledger::LedgerState;
use nomos_mempool::{
    backend::RecoverableMempool, network::NetworkAdapter as MempoolAdapter, DaMempoolService,
    MempoolMsg, TxMempoolService,
};
use nomos_network::{message::ChainSyncEvent, NetworkService};
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend, StorageService};
use nomos_time::{SlotTick, TimeService, TimeServiceMessage};
use overwatch::{
    services::{relay::OutboundRelay, state::StateUpdater, AsServiceId, ServiceCore, ServiceData},
    DynError, OpaqueServiceResourcesHandle,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_with::serde_as;
use services_utils::{
    overwatch::{recovery::backends::FileBackendSettings, JsonFileBackend, RecoveryOperator},
    wait_until_services_are_ready,
};
use thiserror::Error;
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, error, info, instrument, span, Level};
use tracing_futures::Instrument as _;

use crate::{
    leadership::Leader,
    processor::{BlockProcessor as _, NomosBlockProcessor},
    relays::CryptarchiaConsensusRelays,
    states::ChainServiceState,
    storage::{adapters::StorageAdapter, StorageAdapterExt as _},
    sync::block_provider::BlockProvider,
};

type MempoolRelay<Payload, Item, Key> = OutboundRelay<MempoolMsg<HeaderId, Payload, Item, Key>>;
pub(crate) type SamplingRelay<BlobId> = OutboundRelay<DaSamplingServiceMsg<BlobId>>;

// Limit the number of blocks returned by GetHeaders
const HEADERS_LIMIT: usize = 512;
const CRYPTARCHIA_ID: &str = "Cryptarchia";

pub(crate) const LOG_TARGET: &str = "cryptarchia::service";

#[derive(Debug, Error)]
pub enum Error {
    #[error("Ledger error: {0}")]
    Ledger(#[from] nomos_ledger::LedgerError<HeaderId>),
    #[error("Consensus error: {0}")]
    Consensus(#[from] cryptarchia_engine::Error<HeaderId>),
    #[error("Blob is not found from the sampled blobs")]
    SampledBlobNotFound,
    #[error("Failed to validate blobs: {0}")]
    BlobValidationFailure(DynError),
}

#[derive(Clone)]
struct Cryptarchia<State> {
    ledger: nomos_ledger::Ledger<HeaderId>,
    consensus: cryptarchia_engine::Cryptarchia<HeaderId, State>,
}

impl<State: CryptarchiaState> Cryptarchia<State> {
    /// Initialize a new [`Cryptarchia`] instance.
    pub fn from_lib(
        lib_id: HeaderId,
        lib_ledger_state: LedgerState,
        ledger_config: nomos_ledger::Config,
    ) -> Self {
        Self {
            consensus: <cryptarchia_engine::Cryptarchia<_, _>>::from_lib(
                lib_id,
                ledger_config.consensus_config,
            ),
            ledger: <nomos_ledger::Ledger<_>>::new(lib_id, lib_ledger_state, ledger_config),
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

    const fn lib(&self) -> HeaderId {
        self.consensus.lib()
    }

    /// Create a new [`Cryptarchia`] with the updated state.
    #[must_use = "Returns a new instance with the updated state, without modifying the original."]
    fn try_apply_header(&self, header: &Header) -> Result<(Self, PrunedBlocks<HeaderId>), Error> {
        let id = header.id();
        let parent = header.parent();
        let slot = header.slot();
        let ledger = self
            .ledger
            .try_update(id, parent, slot, header.leader_proof())?;
        let (consensus, pruned_blocks) = self.consensus.receive_block(id, parent, slot)?;

        let mut cryptarchia = Self { ledger, consensus };
        // Prune the ledger states of all the pruned blocks.
        cryptarchia.prune_ledger_states(pruned_blocks.all());

        Ok((cryptarchia, pruned_blocks))
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

    /// Remove the ledger states associated with blocks that have been pruned by
    /// the [`cryptarchia_engine::Cryptarchia`].
    ///
    /// Details on which blocks are pruned can be found in the
    /// [`cryptarchia_engine::Cryptarchia::receive_block`].
    fn prune_ledger_states<'a>(&'a mut self, blocks: impl Iterator<Item = &'a HeaderId>) {
        let mut pruned_states_count = 0usize;
        for block in blocks {
            if self.ledger.prune_state_at(block) {
                pruned_states_count = pruned_states_count.saturating_add(1);
            } else {
                tracing::error!(
                   target: LOG_TARGET,
                    "Failed to prune ledger state for block {:?} which should exist.",
                    block
                );
            }
        }
        tracing::debug!(target: LOG_TARGET, "Pruned {pruned_states_count} old forks and their ledger states.");
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CryptarchiaSettings<Ts, Bs, NetworkAdapterSettings, BlendAdapterSettings> {
    #[serde(default)]
    pub transaction_selector_settings: Ts,
    #[serde(default)]
    pub blob_selector_settings: Bs,
    pub config: nomos_ledger::Config,
    pub genesis_id: HeaderId,
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
    SamplingStorage,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
    TimeBackend,
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
    ClPool::Item: Clone + Eq + Debug + 'static,
    ClPool::Key: Debug + 'static,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    DaPool: RecoverableMempool<BlockId = HeaderId>,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Item: Clone + Eq + Debug + 'static,
    DaPool::Key: Debug + 'static,
    DaPool::Settings: Clone,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaPool::Item>,
    BS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = DaPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId>,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
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
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
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
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
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
    ClPool::Item: Clone + Eq + Debug,
    ClPool::Key: Debug,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    DaPool: RecoverableMempool<BlockId = HeaderId>,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Item: Clone + Eq + Debug,
    DaPool::Key: Debug,
    DaPool::Settings: Clone,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaPool::Item>,
    BS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = DaPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId>,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
{
    type Settings = CryptarchiaSettings<
        TxS::Settings,
        BS::Settings,
        NetAdapter::Settings,
        BlendAdapter::Settings,
    >;
    type State = ChainServiceState<
        TxS::Settings,
        BS::Settings,
        NetAdapter::Settings,
        BlendAdapter::Settings,
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
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
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
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        RuntimeServiceId,
    >
where
    NetAdapter: NetworkAdapter<RuntimeServiceId, Block = Block<ClPool::Item, DaPool::Item>>
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
    SamplingBackend: DaSamplingServiceBackend + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + Send + 'static,
    SamplingBackend::BlobId: Debug + Ord + Send + Sync + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<Self>
        + AsServiceId<NetworkService<NetAdapter::Backend, RuntimeServiceId>>
        + AsServiceId<
            BlendService<
                BlendAdapter::Backend,
                BlendAdapter::NodeId,
                BlendAdapter::Network,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<TxMempoolService<ClPoolAdapter, ClPool, RuntimeServiceId>>
        + AsServiceId<
            DaMempoolService<
                DaPoolAdapter,
                DaPool,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<
            DaSamplingService<
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
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
        let mut relays: CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        > = CryptarchiaConsensusRelays::from_service_resources_handle::<_, _, _, _, _>(
            &self.service_resources_handle,
        )
        .await;

        let CryptarchiaSettings {
            config: ledger_config,
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

        // Initialize cryptarchia and leader.
        let cryptarchia = <Cryptarchia<Online>>::from_lib(
            self.initial_state.lib,
            self.initial_state.lib_ledger_state.clone(),
            ledger_config,
        );
        let leader = Leader::new(
            self.initial_state.lib_leader_utxos.clone(),
            leader_config.sk,
            ledger_config,
        );

        // Create a block processor.
        let mut block_processor = NomosBlockProcessor::new(
            relays.cl_mempool_relay().clone(),
            relays.da_mempool_relay().clone(),
            relays.sampling_relay().clone(),
            self.block_subscription_sender.clone(),
        );

        // Recover cryptarchia by loading blocks from the storage.
        let mut cryptarchia = self
            .recovery_cryptarchia(cryptarchia, &mut block_processor, relays.storage_adapter())
            .await?;

        let network_adapter =
            NetAdapter::new(network_adapter_settings, relays.network_relay().clone()).await;
        let tx_selector = TxS::new(transaction_selector_settings);
        let blob_selector = BS::new(blob_selector_settings);

        let mut incoming_blocks = network_adapter.blocks_stream().await?;
        let mut chainsync_events = network_adapter.chainsync_events_stream().await?;
        let sync_blocks_provider: BlockProvider<_, _, _, _> =
            BlockProvider::new(relays.storage_adapter().storage_relay.clone());

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

        self.service_resources_handle.status_updater.notify_ready();
        info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &self.service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _>,
            BlendService<_, _, _, _>,
            TxMempoolService<_, _, _>,
            DaMempoolService<_, _, _, _, _, _, _, _, _>,
            DaSamplingService<_, _, _, _, _, _, _>,
            StorageService<_, _>,
            TimeService<_, _>
        )
        .await?;

        let service_state_updater = self.service_resources_handle.state_updater.clone();

        let async_loop = async {
            loop {
                tokio::select! {
                    Some(block) = incoming_blocks.next() => {
                        Self::log_received_block(&block);

                        cryptarchia = Self::process_block_and_update_service_state(
                            cryptarchia,
                            block,
                            &mut block_processor,
                            relays.storage_adapter(),
                            &service_state_updater,
                            &leader,
                        ).await;

                        info!(counter.consensus_processed_blocks = 1);
                    }

                    Some(SlotTick { slot, .. }) = slot_timer.next() => {
                        let parent = cryptarchia.tip();
                        let aged_tree = cryptarchia.tip_state().aged_commitments();
                        let latest_tree = cryptarchia.tip_state().latest_commitments();
                        debug!("ticking for slot {}", u64::from(slot));

                        let Some(epoch_state) = cryptarchia.epoch_state_for_slot(slot) else {
                            error!("trying to propose a block for slot {} but epoch state is not available", u64::from(slot));
                            continue;
                        };
                        if let Some(proof) = leader.build_proof_for(aged_tree, latest_tree, epoch_state, slot).await {
                            debug!("proposing block...");
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
                                cryptarchia = Self::process_block_and_update_service_state(
                                    cryptarchia,
                                    block.clone(),
                                    &mut block_processor,
                                    relays.storage_adapter(),
                                    &service_state_updater,
                                    &leader,
                                ).await;
                                blend_adapter.blend(block).await;
                            }
                        }
                    }

                    Some(msg) = self.service_resources_handle.inbound_relay.next() => {
                        Self::process_message(&cryptarchia, &self.block_subscription_sender, msg);
                    }

                    Some(event) = chainsync_events.next() => {
                       Self::handle_chainsync_event(&cryptarchia, &sync_blocks_provider, event).await;
                    }
                }
            }
        };

        // It sucks to use `CRYPTARCHIA_ID` when we have `<RuntimeServiceId as
        // AsServiceId<Self>>::SERVICE_ID`.
        // Somehow it just does not let us use it.
        //
        // Hypothesis:
        // 1. Probably related to too many generics.
        // 2. It seems `span` requires a `const` string literal.
        async_loop
            .instrument(span!(Level::TRACE, CRYPTARCHIA_ID))
            .await;

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
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
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
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        RuntimeServiceId,
    >
where
    NetAdapter: NetworkAdapter<RuntimeServiceId> + Clone + Send + Sync + 'static,
    NetAdapter::Settings: Send + Sync,
    BlendAdapter: blend::BlendAdapter<RuntimeServiceId> + Clone + Send + Sync + 'static,
    BlendAdapter::Settings: Send + Sync,
    ClPool: RecoverableMempool<BlockId = HeaderId> + Send + Sync + 'static,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone + Send + Sync + 'static,
    ClPool::Item: Transaction<Hash = ClPool::Key>
        + Debug
        + Clone
        + Eq
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
    TxS::Settings: Send + Sync,
    BS: BlobSelect<BlobId = DaPool::Item> + Clone + Send + Sync + 'static,
    BS::Settings: Send + Sync,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item, DaPool::Item>> + TryInto<Block<ClPool::Item, DaPool::Item>>,
    SamplingBackend: DaSamplingServiceBackend + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + Ord + Send + Sync + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    RuntimeServiceId: 'static,
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
                    error!("Could not send consensus info through channel: {:?}", e);
                });
            }
            ConsensusMsg::BlockSubscribe { sender } => {
                sender.send(block_channel.subscribe()).unwrap_or_else(|_| {
                    error!("Could not subscribe to block subscription channel");
                });
            }
            ConsensusMsg::GetHeaders { from, to, tx } => {
                // default to tip block if not present
                let from = from.unwrap_or_else(|| cryptarchia.tip());
                // default to LIB block if not present
                // TODO: for a full history, we should use genesis, but we don't want to
                // keep it all in memory, headers past LIB should be fetched from storage
                let to = to.unwrap_or_else(|| cryptarchia.lib());

                let mut res = Vec::new();
                let mut cur = from;

                let branches = cryptarchia.consensus.branches();
                while let Some(h) = branches.get(&cur) {
                    res.push(h.id());
                    // limit the response size
                    if cur == to || cur == cryptarchia.lib() || res.len() >= HEADERS_LIMIT {
                        break;
                    }
                    cur = h.parent();
                }

                tx.send(res)
                    .unwrap_or_else(|_| error!("could not send blocks through channel"));
            }
        }
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
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
    ) -> Option<Block<ClPool::Item, DaPool::Item>> {
        let mut output = None;
        let cl_txs = get_mempool_contents(relays.cl_mempool_relay().clone());
        let da_certs = get_mempool_contents(relays.da_mempool_relay().clone());
        let blobs_ids = get_sampled_blobs(relays.sampling_relay());
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
                debug!("proposed block with id {:?}", block.header().id());
                output = Some(block);
            }
            (tx_error, da_certificate_error, blobs_error) => {
                if let Err(_tx_error) = tx_error {
                    error!("Could not fetch block cl transactions");
                }
                if let Err(_da_certificate_error) = da_certificate_error {
                    error!("Could not fetch block da certificates");
                }
                if let Err(_blobs_error) = blobs_error {
                    error!("Could not fetch block da blobs");
                }
            }
        }

        output
    }

    fn log_received_block(block: &Block<ClPool::Item, DaPool::Item>) {
        let content_size = block.header().content_size();
        let transactions = block.cl_transactions_len();
        let blobs = block.bl_blobs_len();

        info!(
            counter.received_blocks = 1,
            transactions = transactions,
            blobs = blobs,
            bytes = content_size
        );
        info!(
            histogram.received_blocks_data = content_size,
            transactions = transactions,
            blobs = blobs
        );
    }

    /// Initialize cryptarchia
    /// It initialize cryptarchia from the LIB (initially genesis) +
    /// (optionally) known blocks which were received before the service
    /// restarted.
    ///
    /// # Arguments
    ///
    /// * `initial_state` - The initial state of cryptarchia.
    /// * `lib_id` - The LIB block id.
    /// * `lib_state` - The LIB ledger state.
    /// * `leader` - The leader instance. It needs to be a Leader initialised to
    ///   genesis. This function will update the leader if needed.
    /// * `ledger_config` - The ledger configuration.
    /// * `relays` - The relays object containing all the necessary relays for
    ///   the consensus.
    /// * `block_subscription_sender` - The broadcast channel to send the blocks
    ///   to the services.
    async fn recovery_cryptarchia<State>(
        &self,
        mut cryptarchia: Cryptarchia<State>,
        block_processor: &mut NomosBlockProcessor<
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            RuntimeServiceId,
        >,
        storage: &mut StorageAdapter<Storage, ClPool::Item, DaPool::Item, RuntimeServiceId>,
    ) -> Result<Cryptarchia<State>, Error>
    where
        State: CryptarchiaState + Send,
    {
        let blocks = storage
            .get_blocks_in_range(cryptarchia.lib(), self.initial_state.tip)
            .await;

        // Skip LIB block since it's already applied
        let blocks = blocks.into_iter().skip(1);

        // Process blocks
        for block in blocks {
            cryptarchia = block_processor
                .process_block(cryptarchia, block, storage)
                .await
                .map_err(|(e, _)| e)?;
        }
        Ok(cryptarchia)
    }

    async fn handle_chainsync_event<State>(
        cryptarchia: &Cryptarchia<State>,
        sync_blocks_provider: &BlockProvider<Storage, State, TxS::Tx, BS::BlobId>,
        event: ChainSyncEvent,
    ) where
        State: CryptarchiaState + Send + Sync + 'static,
    {
        match event {
            ChainSyncEvent::ProvideBlocksRequest {
                target_block,
                local_tip,
                latest_immutable_block,
                additional_blocks,
                reply_sender,
            } => {
                let known_blocks = vec![local_tip, latest_immutable_block]
                    .into_iter()
                    .chain(additional_blocks.into_iter())
                    .collect::<HashSet<_>>();

                sync_blocks_provider
                    .send_blocks(
                        &cryptarchia.consensus,
                        target_block,
                        &known_blocks,
                        reply_sender,
                    )
                    .await;
            }
            ChainSyncEvent::ProvideTipRequest { reply_sender } => {
                let tip = cryptarchia.consensus.tip_branch();
                let response = GetTipResponse {
                    id: tip.id(),
                    slot: tip.slot(),
                };
                info!("Sending tip response: {response:?}");
                if let Err(e) = reply_sender.send(response).await {
                    error!("Failed to send tip header: {e}");
                }
            }
        }
    }

    #[expect(clippy::type_complexity, reason = "StateUpdater")]
    async fn process_block_and_update_service_state<CryptarchiaState>(
        cryptarchia: Cryptarchia<CryptarchiaState>,
        block: Block<ClPool::Item, DaPool::Item>,
        block_processor: &mut NomosBlockProcessor<
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            RuntimeServiceId,
        >,
        storage: &mut StorageAdapter<Storage, ClPool::Item, DaPool::Item, RuntimeServiceId>,
        service_state_updater: &StateUpdater<
            Option<
                ChainServiceState<
                    TxS::Settings,
                    BS::Settings,
                    NetAdapter::Settings,
                    BlendAdapter::Settings,
                >,
            >,
        >,
        leader: &Leader,
    ) -> Cryptarchia<CryptarchiaState>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
    {
        let cryptarchia = match block_processor
            .process_block(cryptarchia, block, storage)
            .await
        {
            Ok(cryptarchia) => cryptarchia,
            Err((e, cryptarchia)) => {
                error!("Failed to process block: {e}");
                cryptarchia
            }
        };

        // Update the service state with the new cryptarchia state.
        match ChainServiceState::new(&cryptarchia, leader, storage.failed_removals().clone()) {
            Ok(state) => {
                service_state_updater.update(Some(state));
            }
            Err(e) => {
                error!("Failed to create service state: {e}");
            }
        }

        cryptarchia
    }
}

#[derive(Debug)]
pub enum ConsensusMsg<Block> {
    Info {
        tx: oneshot::Sender<CryptarchiaInfo>,
    },
    BlockSubscribe {
        sender: oneshot::Sender<broadcast::Receiver<Block>>,
    },
    GetHeaders {
        from: Option<HeaderId>,
        to: Option<HeaderId>,
        tx: oneshot::Sender<Vec<HeaderId>>,
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
) -> Result<Box<dyn Iterator<Item = Item> + Send>, oneshot::error::RecvError>
where
    Key: Send,
    Payload: Send,
{
    let (reply_channel, rx) = oneshot::channel();

    mempool
        .send(MempoolMsg::View {
            ancestor_hint: [0; 32].into(),
            reply_channel,
        })
        .await
        .unwrap_or_else(|(e, _)| eprintln!("Could not get transactions from mempool {e}"));

    rx.await
}

pub(crate) async fn get_sampled_blobs<BlobId>(
    relay: &SamplingRelay<BlobId>,
) -> Result<BTreeSet<BlobId>, DynError>
where
    BlobId: Send,
{
    let (sender, receiver) = oneshot::channel();
    relay
        .send(DaSamplingServiceMsg::GetValidatedBlobs {
            reply_channel: sender,
        })
        .await
        .map_err(|(error, _)| Box::new(error) as DynError)?;
    receiver.await.map_err(|error| Box::new(error) as DynError)
}
