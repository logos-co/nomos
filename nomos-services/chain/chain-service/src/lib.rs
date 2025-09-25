pub mod api;
mod blend;
mod blob;
mod bootstrap;
mod leadership;
mod messages;
pub mod network;
mod relays;
mod states;
pub mod storage;
mod sync;

use core::fmt::Debug;
use std::{
    collections::{BTreeMap, HashSet},
    fmt::Display,
    hash::Hash,
    path::PathBuf,
    time::Duration,
};

use broadcast_service::{BlockBroadcastMsg, BlockBroadcastService, BlockInfo};
use bytes::Bytes;
use cryptarchia_engine::{PrunedBlocks, Slot};
use cryptarchia_sync::{GetTipResponse, ProviderResponse};
use futures::{StreamExt as _, TryFutureExt as _};
pub use leadership::LeaderConfig;
use network::NetworkAdapter;
pub use nomos_blend_service::ServiceComponents as BlendServiceComponents;
use nomos_core::{
    block::Block,
    da,
    header::{Header, HeaderId},
    mantle::{
        AuthenticatedMantleTx, Op, SignedMantleTx, Transaction, TxHash, TxSelect,
        gas::MainnetGasConstants, ops::leader_claim::VoucherCm,
    },
    proofs::leader_proof::Groth16LeaderProof,
};
use nomos_da_sampling::{
    DaSamplingService, DaSamplingServiceMsg, backend::DaSamplingServiceBackend,
};
use nomos_ledger::LedgerState;
use nomos_mempool::{
    MempoolMsg, TxMempoolService, backend::RecoverableMempool,
    network::NetworkAdapter as MempoolAdapter,
};
use nomos_network::{NetworkService, message::ChainSyncEvent};
use nomos_storage::{StorageService, api::chain::StorageChainApi, backends::StorageBackend};
use nomos_time::{SlotTick, TimeService, TimeServiceMessage};
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceCore, ServiceData, relay::OutboundRelay, state::StateUpdater},
};
use relays::BroadcastRelay;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_with::serde_as;
use services_utils::{
    overwatch::{JsonFileBackend, RecoveryOperator, recovery::backends::FileBackendSettings},
    wait_until_services_are_ready,
};
use thiserror::Error;
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::Instant,
};
use tracing::{Level, debug, error, info, instrument, span};
use tracing_futures::Instrument as _;

use crate::{
    blend::BlendAdapter,
    blob::{BlobValidation, RecentBlobValidation, SkipBlobValidation, get_sampled_blobs},
    bootstrap::{
        ibd::{self, InitialBlockDownload},
        state::choose_engine_state,
    },
    leadership::Leader,
    relays::CryptarchiaConsensusRelays,
    states::CryptarchiaConsensusState,
    storage::{StorageAdapter as _, adapters::StorageAdapter},
    sync::{block_provider::BlockProvider, orphan_handler::OrphanBlocksDownloader},
};
pub use crate::{
    bootstrap::config::{BootstrapConfig, IbdConfig, OfflineGracePeriodConfig},
    sync::config::{OrphanConfig, SyncConfig},
};

type MempoolRelay<Payload, Item, Key> = OutboundRelay<MempoolMsg<HeaderId, Payload, Item, Key>>;
type SamplingRelay<BlobId> = OutboundRelay<DaSamplingServiceMsg<BlobId>>;

// Limit the number of blocks returned by GetHeaders
const HEADERS_LIMIT: usize = 512;
const CRYPTARCHIA_ID: &str = "Cryptarchia";

pub(crate) const LOG_TARGET: &str = "cryptarchia::service";

#[derive(Debug, Clone, Error)]
pub enum Error {
    #[error("Ledger error: {0}")]
    Ledger(#[from] nomos_ledger::LedgerError<HeaderId>),
    #[error("Consensus error: {0}")]
    Consensus(#[from] cryptarchia_engine::Error<HeaderId>),
    #[error("Storage error: {0}")]
    Storage(String),
    #[error("Blob validation failed: {0}")]
    BlobValidationFailed(#[from] blob::Error),
}

#[derive(Debug)]
pub enum ConsensusMsg {
    Info {
        tx: oneshot::Sender<CryptarchiaInfo>,
    },
    NewBlockSubscribe {
        sender: oneshot::Sender<broadcast::Receiver<HeaderId>>,
    },
    LibSubscribe {
        sender: oneshot::Sender<broadcast::Receiver<LibUpdate>>,
    },
    GetHeaders {
        from: Option<HeaderId>,
        to: Option<HeaderId>,
        tx: oneshot::Sender<Vec<HeaderId>>,
    },
    GetLedgerState {
        block_id: HeaderId,
        tx: oneshot::Sender<Option<LedgerState>>,
    },
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptarchiaInfo {
    pub lib: HeaderId,
    pub tip: HeaderId,
    pub slot: Slot,
    pub height: u64,
    pub mode: cryptarchia_engine::State,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LibUpdate {
    pub new_lib: HeaderId,
    pub pruned_blocks: PrunedBlocksInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PrunedBlocksInfo {
    pub stale_blocks: Vec<HeaderId>,
    pub immutable_blocks: BTreeMap<Slot, HeaderId>,
}

impl PrunedBlocksInfo {
    /// Returns an iterator over all pruned blocks, both stale and immutable.
    pub fn all(&self) -> impl Iterator<Item = HeaderId> + '_ {
        self.stale_blocks
            .iter()
            .chain(self.immutable_blocks.values())
            .copied()
    }
}

#[derive(Clone)]
struct Cryptarchia {
    ledger: nomos_ledger::Ledger<HeaderId>,
    consensus: cryptarchia_engine::Cryptarchia<HeaderId>,
}

impl Cryptarchia {
    /// Initialize a new [`Cryptarchia`] instance.
    pub fn from_lib(
        lib_id: HeaderId,
        lib_ledger_state: LedgerState,
        ledger_config: nomos_ledger::Config,
        state: cryptarchia_engine::State,
    ) -> Self {
        Self {
            consensus: <cryptarchia_engine::Cryptarchia<_>>::from_lib(
                lib_id,
                ledger_config.consensus_config,
                state,
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
        // A block number of this block if it's applied to the chain.
        let ledger = self.ledger.try_update::<_, MainnetGasConstants>(
            id,
            parent,
            slot,
            header.leader_proof(),
            VoucherCm::default(), // TODO: add the new voucher commitment here
            std::iter::empty::<&SignedMantleTx>(),
        )?;
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

    fn online(self) -> (Self, PrunedBlocks<HeaderId>) {
        let (consensus, pruned_blocks) = self.consensus.online();
        (
            Self {
                ledger: self.ledger,
                consensus,
            },
            pruned_blocks,
        )
    }

    const fn is_boostrapping(&self) -> bool {
        self.consensus.state().is_bootstrapping()
    }

    const fn state(&self) -> &cryptarchia_engine::State {
        self.consensus.state()
    }

    fn has_block(&self, block_id: &HeaderId) -> bool {
        self.consensus.branches().get(block_id).is_some()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CryptarchiaSettings<Ts, NodeId, NetworkAdapterSettings, BlendBroadcastSettings>
where
    NodeId: Clone + Eq + Hash,
{
    #[serde(default)]
    pub transaction_selector_settings: Ts,
    pub config: nomos_ledger::Config,
    pub genesis_id: HeaderId,
    pub genesis_state: LedgerState,
    pub leader_config: LeaderConfig,
    pub network_adapter_settings: NetworkAdapterSettings,
    pub blend_broadcast_settings: BlendBroadcastSettings,
    pub recovery_file: PathBuf,
    pub bootstrap: BootstrapConfig<NodeId>,
    pub sync: SyncConfig,
}

impl<Ts, NodeId, NetworkAdapterSettings, BlendBroadcastSettings> FileBackendSettings
    for CryptarchiaSettings<Ts, NodeId, NetworkAdapterSettings, BlendBroadcastSettings>
where
    NodeId: Clone + Eq + Hash,
{
    fn recovery_file(&self) -> &PathBuf {
        &self.recovery_file
    }
}

#[expect(clippy::allow_attributes_without_reason)]
pub struct CryptarchiaConsensus<
    NetAdapter,
    BlendService,
    ClPool,
    ClPoolAdapter,
    TxS,
    Storage,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
> where
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Backend: 'static,
    NetAdapter::Settings: Send,
    NetAdapter::PeerId: Clone + Eq + Hash,
    BlendService: nomos_blend_service::ServiceComponents,
    ClPool: RecoverableMempool<BlockId = HeaderId, Key = TxHash>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone,
    ClPool::Item: Clone + Eq + Debug + 'static,
    ClPool::Item: AuthenticatedMantleTx,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    new_block_subscription_sender: broadcast::Sender<HeaderId>,
    lib_subscription_sender: broadcast::Sender<LibUpdate>,
    initial_state: <Self as ServiceData>::State,
}

impl<
    NetAdapter,
    BlendService,
    ClPool,
    ClPoolAdapter,
    TxS,
    Storage,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
> ServiceData
    for CryptarchiaConsensus<
        NetAdapter,
        BlendService,
        ClPool,
        ClPoolAdapter,
        TxS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        RuntimeServiceId,
    >
where
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Settings: Send,
    NetAdapter::PeerId: Clone + Eq + Hash,
    BlendService: nomos_blend_service::ServiceComponents,
    ClPool: RecoverableMempool<BlockId = HeaderId, Key = TxHash>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone,
    ClPool::Item: AuthenticatedMantleTx + Clone + Eq + Debug,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
{
    type Settings = CryptarchiaSettings<
        TxS::Settings,
        NetAdapter::PeerId,
        NetAdapter::Settings,
        BlendService::BroadcastSettings,
    >;
    type State = CryptarchiaConsensusState<
        TxS::Settings,
        NetAdapter::PeerId,
        NetAdapter::Settings,
        BlendService::BroadcastSettings,
    >;
    type StateOperator = RecoveryOperator<JsonFileBackend<Self::State, Self::Settings>>;
    type Message = ConsensusMsg;
}

#[async_trait::async_trait]
impl<
    NetAdapter,
    BlendService,
    ClPool,
    ClPoolAdapter,
    TxS,
    Storage,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
> ServiceCore<RuntimeServiceId>
    for CryptarchiaConsensus<
        NetAdapter,
        BlendService,
        ClPool,
        ClPoolAdapter,
        TxS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        RuntimeServiceId,
    >
where
    NetAdapter: NetworkAdapter<RuntimeServiceId, Block = Block<ClPool::Item>>
        + Clone
        + Send
        + Sync
        + 'static,
    NetAdapter::Settings: Send + Sync + 'static,
    NetAdapter::PeerId: Clone + Eq + Hash + Copy + Debug + Send + Sync + Unpin + 'static,
    BlendService: ServiceData<
            Message = nomos_blend_service::message::ServiceMessage<BlendService::BroadcastSettings>,
        > + nomos_blend_service::ServiceComponents
        + Send
        + Sync
        + 'static,
    BlendService::BroadcastSettings: Clone + Send + Sync,
    ClPool: RecoverableMempool<BlockId = HeaderId, Key = TxHash> + Send + Sync + 'static,
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
        + Unpin
        + 'static,
    ClPool::Item: AuthenticatedMantleTx,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>
        + Send
        + Sync
        + 'static,
    TxS: TxSelect<Tx = ClPool::Item> + Clone + Send + Sync + 'static,
    TxS::Settings: Send + Sync + 'static,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item>> + TryInto<Block<ClPool::Item>> + Into<Bytes>,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + Send + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<Self>
        + AsServiceId<NetworkService<NetAdapter::Backend, RuntimeServiceId>>
        + AsServiceId<BlendService>
        + AsServiceId<BlockBroadcastService<RuntimeServiceId>>
        + AsServiceId<
            TxMempoolService<
                ClPoolAdapter,
                SamplingNetworkAdapter,
                SamplingStorage,
                ClPool,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<
            DaSamplingService<
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
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
        let (new_block_subscription_sender, _) = broadcast::channel(16);
        let (lib_subscription_sender, _) = broadcast::channel(16);

        Ok(Self {
            service_resources_handle,
            new_block_subscription_sender,
            lib_subscription_sender,
            initial_state,
        })
    }

    #[expect(clippy::too_many_lines, reason = "TODO: Address this at some point.")]
    async fn run(mut self) -> Result<(), DynError> {
        let relays: CryptarchiaConsensusRelays<
            BlendService,
            ClPool,
            ClPoolAdapter,
            NetAdapter,
            SamplingBackend,
            Storage,
            TxS,
            RuntimeServiceId,
        > = CryptarchiaConsensusRelays::from_service_resources_handle::<_, _, _>(
            &self.service_resources_handle,
        )
        .await;

        let CryptarchiaSettings {
            config: ledger_config,
            genesis_id,
            transaction_selector_settings,
            leader_config,
            network_adapter_settings,
            blend_broadcast_settings,
            bootstrap: bootstrap_config,
            sync: sync_config,
            ..
        } = self
            .service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        // TODO: check active slot coeff is exactly 1/30

        // These are blocks that have been pruned by the cryptarchia engine but have not
        // yet been deleted from the storage layer.
        let storage_blocks_to_remove = self.initial_state.storage_blocks_to_remove.clone();
        let (cryptarchia, pruned_blocks, leader) = self
            .initialize_cryptarchia(
                genesis_id,
                &bootstrap_config,
                ledger_config,
                leader_config,
                &relays,
            )
            .await;
        let storage_blocks_to_remove = Self::delete_pruned_blocks_from_storage(
            pruned_blocks.stale_blocks().copied(),
            &storage_blocks_to_remove,
            relays.storage_adapter(),
        )
        .await;

        let network_adapter =
            NetAdapter::new(network_adapter_settings, relays.network_relay().clone()).await;
        let tx_selector = TxS::new(transaction_selector_settings);

        let mut incoming_blocks = network_adapter.blocks_stream().await?;
        let mut chainsync_events = network_adapter.chainsync_events_stream().await?;
        let sync_blocks_provider: BlockProvider<_, _> =
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

        let blend_adapter = BlendAdapter::<BlendService>::new(
            relays.blend_relay().clone(),
            blend_broadcast_settings.clone(),
        );

        let mut orphan_downloader = Box::pin(OrphanBlocksDownloader::new(
            network_adapter.clone(),
            sync_config.orphan.max_orphan_cache_size,
        ));

        self.service_resources_handle.status_updater.notify_ready();
        info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &self.service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkService<_, _>,
            BlendService,
            TxMempoolService<_, _, _, _, _>,
            DaSamplingService<_, _, _, _>,
            StorageService<_, _>,
            TimeService<_, _>
        )
        .await?;

        // Run IBD (Initial Block Download).
        // TODO: Currently, we're passing a closure that processes each block.
        //       It needs to be replaced with a trait, which requires substantial
        // refactoring.       https://github.com/logos-co/nomos/issues/1505
        let initial_block_download = InitialBlockDownload::new(
            bootstrap_config.ibd,
            cryptarchia,
            storage_blocks_to_remove,
            network_adapter,
            |cryptarchia, storage_blocks_to_remove, block| {
                let leader = &leader;
                let relays = &relays;
                let state_updater = &self.service_resources_handle.state_updater;
                let new_block_subscription_sender = &self.new_block_subscription_sender;
                let lib_subscription_sender = &self.lib_subscription_sender;
                async move {
                    Self::process_block_and_update_state(
                        cryptarchia,
                        leader,
                        block,
                        // TODO: Enable this once entering DA window: https://github.com/logos-co/nomos/issues/1675
                        &SkipBlobValidation,
                        &storage_blocks_to_remove,
                        relays,
                        new_block_subscription_sender,
                        lib_subscription_sender,
                        state_updater,
                    )
                    .await
                    .map_err(|e| {
                        error!("Error processing block during IBD: {:?}", e);
                        ibd::Error::from(e)
                    })
                }
            },
        );

        let (mut cryptarchia, mut storage_blocks_to_remove) = match initial_block_download
            .run()
            .await
        {
            Ok((cryptarchia, storage_blocks_to_remove)) => {
                info!("Initial Block Download completed successfully.");
                (cryptarchia, storage_blocks_to_remove)
            }
            Err(e) => {
                error!("Initial Block Download failed: {e:?}. Initiating graceful shutdown.");

                if let Err(shutdown_err) = self
                    .service_resources_handle
                    .overwatch_handle
                    .shutdown()
                    .await
                {
                    error!("Failed to shutdown overwatch: {shutdown_err:?}");
                }

                error!(
                    "Initial Block Download did not complete successfully: {e}. Common causes: unresponsive initial peers, \
                network issues, or incorrect peer addresses. Consider retrying with different bootstrap peers."
                );

                return Err(DynError::from(format!(
                    "Initial Block Download failed: {e:?}"
                )));
            }
        };

        // Start the timer for Prolonged Bootstrap Period.
        let mut prolonged_bootstrap_timer = Box::pin(tokio::time::sleep_until(
            Instant::now() + bootstrap_config.prolonged_bootstrap_period,
        ));

        // Start the timer for periodic state recording for offline grace period
        let mut state_recording_timer = tokio::time::interval(
            bootstrap_config
                .offline_grace_period
                .state_recording_interval,
        );

        let mut blob_validation: Box<dyn BlobValidation<_, _> + Send + Sync> =
            if cryptarchia.is_boostrapping() {
                Box::new(SkipBlobValidation)
            } else {
                Box::new(RecentBlobValidation::new(relays.sampling_relay().clone()))
            };

        let async_loop = async {
            loop {
                tokio::select! {
                    () = &mut prolonged_bootstrap_timer, if cryptarchia.is_boostrapping() => {
                        info!("Prolonged Bootstrap Period has passed. Switching to Online.");
                        (cryptarchia, storage_blocks_to_remove) = Self::switch_to_online(
                            cryptarchia,
                            &storage_blocks_to_remove,
                            relays.storage_adapter(),
                        ).await;
                        blob_validation = Box::new(RecentBlobValidation::new(relays.sampling_relay().clone()));
                        Self::update_state(
                            &cryptarchia,
                            &leader,
                            storage_blocks_to_remove.clone(),
                            &self.service_resources_handle.state_updater,
                        );
                    }

                    Some(block) = incoming_blocks.next() => {
                        Self::log_received_block(&block);

                        if cryptarchia.has_block(&block.header().id()) {
                            info!(target: LOG_TARGET, "Block {:?} already processed, ignoring", block.header().id());
                            continue;
                        }

                        // Process the received block and update the cryptarchia state.
                        match Self::process_block_and_update_state(
                            cryptarchia.clone(),
                            &leader,
                            block.clone(),
                            blob_validation.as_ref(),
                            &storage_blocks_to_remove,
                            &relays,
                            &self.new_block_subscription_sender,
                            &self.lib_subscription_sender,
                            &self.service_resources_handle.state_updater
                        ).await {
                            Ok((new_cryptarchia, new_storage_blocks_to_remove)) => {
                                cryptarchia = new_cryptarchia;
                                storage_blocks_to_remove = new_storage_blocks_to_remove;

                                orphan_downloader.remove_orphan(&block.header().id());

                                info!(counter.consensus_processed_blocks = 1);
                            }
                            Err(Error::Ledger(nomos_ledger::LedgerError::ParentNotFound(parent))
                                | Error::Consensus(cryptarchia_engine::Error::ParentMissing(parent)),) => {

                                orphan_downloader.enqueue_orphan(block.header().id(), cryptarchia.tip(), cryptarchia.lib());

                                error!(target: LOG_TARGET, "Received block with parent {:?} that is not in the ledger state. Ignoring block.", parent);
                            }
                            Err(e) => {
                                error!(target: LOG_TARGET, "Error processing block: {:?}", e);
                            }
                        }
                    }

                    Some(SlotTick { slot, .. }) = slot_timer.next() => {
                        // TODO: Don't propose blocks until IBD is done and online mode is activated.
                        //       Until then, mempool, DA, blend service will not be ready: https://github.com/logos-co/nomos/issues/1656
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
                                &relays
                            ).await;

                            if let Some(block) = block {
                                // apply our own block
                                match Self::process_block_and_update_state(
                                    cryptarchia.clone(),
                                    &leader,
                                    block.clone(),
                                    // Skip this since the block was already built with valid blobs.
                                    &SkipBlobValidation,
                                    &storage_blocks_to_remove,
                                    &relays,
                                    &self.new_block_subscription_sender,
                                    &self.lib_subscription_sender,
                                    &self.service_resources_handle.state_updater,
                                )
                                .await {
                                    Ok((new_cryptarchia, new_storage_blocks_to_remove)) => {
                                        cryptarchia = new_cryptarchia;
                                        storage_blocks_to_remove = new_storage_blocks_to_remove;

                                        blend_adapter.publish_block(
                                    block,
                                ).await;
                                    }
                                    Err(e) => {
                                        error!(target: LOG_TARGET, "Error processing local block: {:?}", e);
                                    }
                                }
                            }
                        }
                    }

                    Some(msg) = self.service_resources_handle.inbound_relay.next() => {
                        Self::process_message(&cryptarchia, &self.new_block_subscription_sender, &self.lib_subscription_sender, msg);
                    }

                    Some(event) = chainsync_events.next() => {
                        if cryptarchia.state().is_online() {
                           Self::handle_chainsync_event(&cryptarchia, &sync_blocks_provider, event).await;
                        } else {
                            Self::reject_chain_sync_event(event).await;
                        }
                    }

                    Some(block) = orphan_downloader.next(), if orphan_downloader.should_poll() => {
                        let header_id= block.header().id();
                        info!("Processing block from orphan downloader: {header_id:?}");

                        if cryptarchia.has_block(&block.header().id()) {
                            continue;
                        }

                        match Self::process_block_and_update_state(
                            cryptarchia.clone(),
                            &leader,
                            block.clone(),
                            blob_validation.as_ref(),
                            &storage_blocks_to_remove,
                            &relays,
                            &self.new_block_subscription_sender,
                            &self.lib_subscription_sender,
                            &self.service_resources_handle.state_updater
                        ).await {
                            Ok((new_cryptarchia, new_storage_blocks_to_remove)) => {
                                cryptarchia = new_cryptarchia;
                                storage_blocks_to_remove = new_storage_blocks_to_remove;

                                info!(counter.consensus_processed_blocks = 1);
                            }
                            Err(e) => {
                                error!(target: LOG_TARGET, "Error processing orphan downloader block: {e:?}");
                                orphan_downloader.cancel_active_download();
                            }
                        }
                    }

                    _ = state_recording_timer.tick() => {
                        // Periodically record the current timestamp and engine state
                        Self::update_state(
                            &cryptarchia,
                            &leader,
                            storage_blocks_to_remove.clone(),
                            &self.service_resources_handle.state_updater,
                        );
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
    BlendService,
    ClPool,
    ClPoolAdapter,
    TxS,
    Storage,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
>
    CryptarchiaConsensus<
        NetAdapter,
        BlendService,
        ClPool,
        ClPoolAdapter,
        TxS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        RuntimeServiceId,
    >
where
    NetAdapter: NetworkAdapter<RuntimeServiceId, Block = Block<ClPool::Item>>
        + Clone
        + Send
        + Sync
        + 'static,
    NetAdapter::Settings: Send + Sync + 'static,
    NetAdapter::PeerId: Clone + Eq + Hash + Copy + Debug + Send + Sync,
    BlendService: ServiceData<
            Message = nomos_blend_service::message::ServiceMessage<BlendService::BroadcastSettings>,
        > + nomos_blend_service::ServiceComponents
        + Send
        + Sync
        + 'static,
    BlendService::BroadcastSettings: Send + Sync,
    ClPool: RecoverableMempool<BlockId = HeaderId, Key = TxHash> + Send + Sync + 'static,
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
    ClPool::Item: AuthenticatedMantleTx,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>
        + Send
        + Sync
        + 'static,
    TxS: TxSelect<Tx = ClPool::Item> + Clone + Send + Sync + 'static,
    TxS::Settings: Send + Sync + 'static,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item>> + TryInto<Block<ClPool::Item>> + Into<Bytes>,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
{
    fn process_message(
        cryptarchia: &Cryptarchia,
        new_block_channel: &broadcast::Sender<HeaderId>,
        lib_channel: &broadcast::Sender<LibUpdate>,
        msg: ConsensusMsg,
    ) {
        match msg {
            ConsensusMsg::Info { tx } => {
                let info = CryptarchiaInfo {
                    lib: cryptarchia.lib(),
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
                    mode: *cryptarchia.consensus.state(),
                };
                tx.send(info).unwrap_or_else(|e| {
                    error!("Could not send consensus info through channel: {:?}", e);
                });
            }
            ConsensusMsg::NewBlockSubscribe { sender } => {
                sender
                    .send(new_block_channel.subscribe())
                    .unwrap_or_else(|_| {
                        error!("Could not subscribe to new block channel");
                    });
            }
            ConsensusMsg::LibSubscribe { sender } => {
                sender.send(lib_channel.subscribe()).unwrap_or_else(|_| {
                    error!("Could not subscribe to LIB updates channel");
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
            ConsensusMsg::GetLedgerState { block_id, tx } => {
                let ledger_state = cryptarchia.ledger.state(&block_id).cloned();
                tx.send(ledger_state).unwrap_or_else(|_| {
                    error!("Could not send ledger state through channel");
                });
            }
        }
    }

    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusState and CryptarchiaConsensusRelays amount of generics."
    )]
    #[expect(
        clippy::too_many_arguments,
        reason = "This function does too much, need to deal with this at some point"
    )]
    async fn process_block_and_update_state(
        cryptarchia: Cryptarchia,
        leader: &Leader,
        block: Block<ClPool::Item>,
        blob_validation: &(impl BlobValidation<SamplingBackend::BlobId, ClPool::Item> + Sync + ?Sized),
        storage_blocks_to_remove: &HashSet<HeaderId>,
        relays: &CryptarchiaConsensusRelays<
            BlendService,
            ClPool,
            ClPoolAdapter,
            NetAdapter,
            SamplingBackend,
            Storage,
            TxS,
            RuntimeServiceId,
        >,
        new_block_subscription_sender: &broadcast::Sender<HeaderId>,
        lib_subscription_sender: &broadcast::Sender<LibUpdate>,
        state_updater: &StateUpdater<
            Option<
                CryptarchiaConsensusState<
                    TxS::Settings,
                    NetAdapter::PeerId,
                    NetAdapter::Settings,
                    BlendService::BroadcastSettings,
                >,
            >,
        >,
    ) -> Result<(Cryptarchia, HashSet<HeaderId>), Error> {
        let (cryptarchia, pruned_blocks) = Self::process_block(
            cryptarchia,
            block,
            blob_validation,
            relays,
            new_block_subscription_sender,
            lib_subscription_sender,
        )
        .await?;

        let storage_blocks_to_remove = Self::delete_pruned_blocks_from_storage(
            pruned_blocks.stale_blocks().copied(),
            storage_blocks_to_remove,
            relays.storage_adapter(),
        )
        .await;

        Self::update_state(
            &cryptarchia,
            leader,
            storage_blocks_to_remove.clone(),
            state_updater,
        );

        Ok((cryptarchia, storage_blocks_to_remove))
    }

    #[expect(clippy::type_complexity, reason = "StateUpdater")]
    fn update_state(
        cryptarchia: &Cryptarchia,
        leader: &Leader,
        storage_blocks_to_remove: HashSet<HeaderId>,
        state_updater: &StateUpdater<
            Option<
                CryptarchiaConsensusState<
                    TxS::Settings,
                    NetAdapter::PeerId,
                    NetAdapter::Settings,
                    BlendService::BroadcastSettings,
                >,
            >,
        >,
    ) {
        match <Self as ServiceData>::State::from_cryptarchia_and_unpruned_blocks(
            cryptarchia,
            leader,
            storage_blocks_to_remove,
        ) {
            Ok(state) => {
                state_updater.update(Some(state));
            }
            Err(e) => {
                error!(target: LOG_TARGET, "Failed to update state: {}", e);
            }
        }
    }

    /// Try to add a [`Block`] to [`Cryptarchia`].
    /// A [`Block`] is only added if it's valid
    #[expect(clippy::allow_attributes_without_reason)]
    #[instrument(level = "debug", skip(cryptarchia, relays, blob_validation))]
    async fn process_block(
        cryptarchia: Cryptarchia,
        block: Block<ClPool::Item>,
        blob_validation: &(impl BlobValidation<SamplingBackend::BlobId, ClPool::Item> + Sync + ?Sized),
        relays: &CryptarchiaConsensusRelays<
            BlendService,
            ClPool,
            ClPoolAdapter,
            NetAdapter,
            SamplingBackend,
            Storage,
            TxS,
            RuntimeServiceId,
        >,
        new_block_subscription_sender: &broadcast::Sender<HeaderId>,
        lib_broadcaster: &broadcast::Sender<LibUpdate>,
    ) -> Result<(Cryptarchia, PrunedBlocks<HeaderId>), Error> {
        debug!("received proposal {:?}", block);

        blob_validation.validate(&block).await?;

        // TODO: filter on time?
        let header = block.header();
        let id = header.id();
        let prev_lib = cryptarchia.lib();

        let (cryptarchia, pruned_blocks) = cryptarchia.try_apply_header(header)?;
        let new_lib = cryptarchia.lib();

        // remove included content from mempool
        mark_in_block(
            relays.cl_mempool_relay().clone(),
            block.transactions().map(Transaction::hash),
            id,
        )
        .await;

        mark_blob_in_block(
            relays.sampling_relay().clone(),
            vec![], // TODO: pass actual blob ids here
        )
        .await;

        relays
            .storage_adapter()
            .store_block(header.id(), block.clone())
            .await
            .map_err(|e| Error::Storage(format!("Failed to store block: {e}")))?;

        let immutable_blocks = pruned_blocks.immutable_blocks().clone();
        relays
            .storage_adapter()
            .store_immutable_block_ids(immutable_blocks)
            .await
            .map_err(|e| Error::Storage(format!("Failed to store immutable block ids: {e}")))?;

        if let Err(e) = new_block_subscription_sender.send(header.id()) {
            error!("Could not notify new block to services {e}");
        }

        if prev_lib != new_lib {
            let height = cryptarchia
                .consensus
                .branches()
                .get(&cryptarchia.lib())
                .expect("LIB branch not available")
                .length();
            let block_info = BlockInfo {
                height,
                header_id: new_lib,
            };
            if let Err(e) = broadcast_finalized_block(relays.broadcast_relay(), block_info).await {
                error!("Could not notify block to services {e}");
            }

            let lib_update = LibUpdate {
                new_lib: cryptarchia.lib(),
                pruned_blocks: PrunedBlocksInfo {
                    stale_blocks: pruned_blocks.stale_blocks().copied().collect(),
                    immutable_blocks: pruned_blocks.immutable_blocks().clone(),
                },
            };

            if let Err(e) = lib_broadcaster.send(lib_update) {
                error!("Could not notify LIB update to services: {e}");
            }
        }

        Ok((cryptarchia, pruned_blocks))
    }

    #[expect(clippy::allow_attributes_without_reason)]
    #[instrument(level = "debug", skip(tx_selector, relays))]
    async fn propose_block(
        parent: HeaderId,
        slot: Slot,
        proof: Groth16LeaderProof,
        tx_selector: TxS,
        relays: &CryptarchiaConsensusRelays<
            BlendService,
            ClPool,
            ClPoolAdapter,
            NetAdapter,
            SamplingBackend,
            Storage,
            TxS,
            RuntimeServiceId,
        >,
    ) -> Option<Block<ClPool::Item>> {
        let mut output = None;
        let txs = get_mempool_contents(relays.cl_mempool_relay().clone()).map_err(DynError::from);
        let sampling_relay = relays.sampling_relay().clone();
        let blobs_ids = get_sampled_blobs(&sampling_relay);
        match futures::try_join!(txs, blobs_ids) {
            Ok((txs, blobs)) => {
                let txs = tx_selector
                    .select_tx_from(txs.filter(|tx|
                    // skip txs that try to include a blob which is not yet sampled
                    tx.mantle_tx().ops.iter().all(|op| match op {
                        Op::ChannelBlob(op) => blobs.contains(&op.blob),
                        _ => true,
                    })))
                    .collect::<Vec<_>>();
                let content_id = [0; 32].into(); // TODO: calculate the actual content id
                // TODO: this should probably be a proposal or be transformed into a proposal
                let block = Block::new(Header::new(parent, content_id, slot, proof), txs);
                debug!("proposed block with id {:?}", block.header().id());
                output = Some(block);
            }
            Err(e) => {
                error!("Could not fetch block transactions: {e}");
            }
        }

        output
    }
    fn log_received_block(block: &Block<ClPool::Item>) {
        let content_size = 0; // TODO: calculate the actual content size
        let transactions = block.transactions().len();

        info!(
            counter.received_blocks = 1,
            transactions = transactions,
            bytes = content_size
        );
        info!(
            histogram.received_blocks_data = content_size,
            transactions = transactions,
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
        storage_adapter: &StorageAdapter<Storage, TxS::Tx, RuntimeServiceId>,
    ) -> Vec<Block<ClPool::Item>> {
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
    async fn initialize_cryptarchia(
        &self,
        genesis_id: HeaderId,
        bootstrap_config: &BootstrapConfig<NetAdapter::PeerId>,
        ledger_config: nomos_ledger::Config,
        leader_config: LeaderConfig,
        relays: &CryptarchiaConsensusRelays<
            BlendService,
            ClPool,
            ClPoolAdapter,
            NetAdapter,
            SamplingBackend,
            Storage,
            TxS,
            RuntimeServiceId,
        >,
    ) -> (Cryptarchia, PrunedBlocks<HeaderId>, Leader) {
        let lib_id = self.initial_state.lib;
        let state = choose_engine_state(
            lib_id,
            genesis_id,
            bootstrap_config,
            self.initial_state.last_engine_state.as_ref(),
        );
        let mut cryptarchia = Cryptarchia::from_lib(
            lib_id,
            self.initial_state.lib_ledger_state.clone(),
            ledger_config,
            state,
        );
        let leader = Leader::new(
            self.initial_state.lib_leader_utxos.clone(),
            leader_config.sk,
            ledger_config,
        );

        let blocks =
            Self::get_blocks_in_range(lib_id, self.initial_state.tip, relays.storage_adapter())
                .await;

        // Skip LIB block since it's already applied
        let blocks = blocks.into_iter().skip(1);

        let mut pruned_blocks = PrunedBlocks::new();
        for block in blocks {
            match Self::process_block(
                cryptarchia.clone(),
                block,
                &SkipBlobValidation,
                relays,
                &self.new_block_subscription_sender,
                &self.lib_subscription_sender,
            )
            .await
            {
                Ok((new_cryptarchia, new_pruned_blocks)) => {
                    cryptarchia = new_cryptarchia;
                    pruned_blocks.extend(&new_pruned_blocks);
                }
                Err(e) => {
                    error!(target: LOG_TARGET, "Error processing block: {:?}", e);
                }
            }
        }

        (cryptarchia, pruned_blocks, leader)
    }

    /// Remove the pruned blocks from the storage layer.
    ///
    /// Also, this removes the `additional_blocks` from the storage
    /// layer. These blocks might belong to previous pruning operations and
    /// that failed to be removed from the storage for some reason.
    ///
    /// This function returns any block that fails to be deleted from the
    /// storage layer.
    async fn delete_pruned_blocks_from_storage(
        pruned_blocks: impl Iterator<Item = HeaderId> + Send,
        additional_blocks: &HashSet<HeaderId>,
        storage_adapter: &StorageAdapter<Storage, TxS::Tx, RuntimeServiceId>,
    ) -> HashSet<HeaderId> {
        match Self::delete_blocks_from_storage(
            pruned_blocks.chain(additional_blocks.iter().copied()),
            storage_adapter,
        )
        .await
        {
            // No blocks failed to be deleted.
            Ok(()) => HashSet::new(),
            // We retain the blocks that failed to be deleted.
            Err(failed_blocks) => failed_blocks
                .into_iter()
                .map(|(block_id, _)| block_id)
                .collect(),
        }
    }

    /// Send a bulk blocks deletion request to the storage adapter.
    ///
    /// If no request fails, the method returns `Ok()`.
    /// If any request fails, the header ID and the generated error for each
    /// failing request are collected and returned as part of the `Err`
    /// result.
    async fn delete_blocks_from_storage<Headers>(
        block_headers: Headers,
        storage_adapter: &StorageAdapter<Storage, TxS::Tx, RuntimeServiceId>,
    ) -> Result<(), Vec<(HeaderId, DynError)>>
    where
        Headers: Iterator<Item = HeaderId> + Send,
    {
        let blocks_to_delete = block_headers.collect::<Vec<_>>();
        let block_deletion_outcomes = blocks_to_delete.iter().copied().zip(
            storage_adapter
                .remove_blocks(blocks_to_delete.iter().copied())
                .await,
        );

        let errors: Vec<_> = block_deletion_outcomes
            .filter_map(|(block_id, outcome)| match outcome {
                Ok(Some(_)) => {
                    tracing::debug!(
                        target: LOG_TARGET,
                        "Block {block_id:#?} successfully deleted from storage."
                    );
                    None
                }
                Ok(None) => {
                    tracing::trace!(
                        target: LOG_TARGET,
                        "Block {block_id:#?} was not found in storage."
                    );
                    None
                }
                Err(e) => {
                    tracing::error!(
                        target: LOG_TARGET,
                        "Error deleting block {block_id:#?} from storage: {e}."
                    );
                    Some((block_id, e))
                }
            })
            .collect();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    async fn handle_chainsync_event(
        cryptarchia: &Cryptarchia,
        sync_blocks_provider: &BlockProvider<Storage, TxS::Tx>,
        event: ChainSyncEvent,
    ) {
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
                let response = ProviderResponse::Available(GetTipResponse::Tip {
                    tip: tip.id(),
                    slot: tip.slot(),
                    height: tip.length(),
                });

                info!("Sending tip response: {response:?}");
                if let Err(e) = reply_sender.send(response).await {
                    error!("Failed to send tip header: {e}");
                }
            }
        }
    }

    async fn reject_chain_sync_event(event: ChainSyncEvent) {
        debug!(target: LOG_TARGET, "Received chainsync event while in bootstrapping state. Ignoring it.");
        match event {
            ChainSyncEvent::ProvideBlocksRequest { reply_sender, .. } => {
                Self::send_chain_sync_rejection(reply_sender).await;
            }
            ChainSyncEvent::ProvideTipRequest { reply_sender } => {
                Self::send_chain_sync_rejection(reply_sender).await;
            }
        }
    }

    async fn send_chain_sync_rejection<ResponseType>(
        sender: mpsc::Sender<ProviderResponse<ResponseType>>,
    ) {
        let response = ProviderResponse::Unavailable {
            reason: "Node is not in online mode".to_owned(),
        };
        if let Err(e) = sender.send(response).await {
            error!("Failed to send chain sync response: {e}");
        }
    }

    async fn switch_to_online(
        cryptarchia: Cryptarchia,
        storage_blocks_to_remove: &HashSet<HeaderId>,
        storage_adapter: &StorageAdapter<Storage, TxS::Tx, RuntimeServiceId>,
    ) -> (Cryptarchia, HashSet<HeaderId>) {
        let (cryptarchia, pruned_blocks) = cryptarchia.online();
        if let Err(e) = storage_adapter
            .store_immutable_block_ids(pruned_blocks.immutable_blocks().clone())
            .await
        {
            error!("Could not store immutable block IDs: {e}");
        }

        let storage_blocks_to_remove = Self::delete_pruned_blocks_from_storage(
            pruned_blocks.stale_blocks().copied(),
            storage_blocks_to_remove,
            storage_adapter,
        )
        .await;

        (cryptarchia, storage_blocks_to_remove)
    }
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
        .unwrap_or_else(|(e, _)| error!("Could not mark items in block: {e}"));
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

async fn broadcast_finalized_block(
    broadcast_relay: &BroadcastRelay,
    block_info: BlockInfo,
) -> Result<(), DynError> {
    broadcast_relay
        .send(BlockBroadcastMsg::BroadcastFinalizedBlock(block_info))
        .await
        .map_err(|(error, _)| Box::new(error) as DynError)
}
