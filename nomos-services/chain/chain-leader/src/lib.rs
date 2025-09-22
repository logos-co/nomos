mod blend;
mod bootstrap;
mod leadership;
mod messages;
pub mod network;
mod relays;
pub mod storage;

use core::fmt::Debug;
use std::{
    collections::{BTreeSet, HashSet},
    fmt::Display,
    hash::Hash,
    path::PathBuf,
    time::Duration,
};

use cryptarchia_engine::{PrunedBlocks, Slot};
use futures::{StreamExt as _, TryFutureExt as _};
pub use leadership::LeaderConfig;
use network::NetworkAdapter;
pub use nomos_blend_service::ServiceComponents as BlendServiceComponents;
use nomos_core::{
    block::Block,
    da,
    header::{Header, HeaderId},
    mantle::{
        gas::MainnetGasConstants, ops::leader_claim::VoucherCm, AuthenticatedMantleTx, Op,
        SignedMantleTx, Transaction, TxHash, TxSelect,
    },
    proofs::leader_proof::Groth16LeaderProof,
};
use nomos_da_sampling::{
    backend::DaSamplingServiceBackend, DaSamplingService, DaSamplingServiceMsg,
};
use nomos_ledger::LedgerState;
use nomos_mempool::{
    backend::RecoverableMempool, network::NetworkAdapter as MempoolAdapter, MempoolMsg,
    TxMempoolService,
};
use nomos_network::NetworkService;
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend, StorageService};
use nomos_time::{SlotTick, TimeService, TimeServiceMessage};
use overwatch::{
    services::{relay::OutboundRelay, AsServiceId, ServiceCore, ServiceData},
    DynError, OpaqueServiceResourcesHandle,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use thiserror::Error;
use tokio::sync::oneshot;
use tracing::{debug, error, info, instrument, span, Level};
use tracing_futures::Instrument as _;

use crate::{
    blend::BlendAdapter,
    bootstrap::{ibd::InitialBlockDownload, state::choose_engine_state},
    leadership::Leader,
    relays::CryptarchiaConsensusRelays,
    storage::{adapters::StorageAdapter, StorageAdapter as _},
};
pub use crate::{
    bootstrap::config::{BootstrapConfig, IbdConfig, OfflineGracePeriodConfig},
};

type MempoolRelay<Payload, Item, Key> = OutboundRelay<MempoolMsg<HeaderId, Payload, Item, Key>>;
type SamplingRelay<BlobId> = OutboundRelay<DaSamplingServiceMsg<BlobId>>;

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
}

#[derive(Debug)]
pub enum LeaderMsg {
    // Empty enum - leader service doesn't handle external messages
    // Block production is driven by slot ticks, not messages
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


    fn has_block(&self, block_id: &HeaderId) -> bool {
        self.consensus.branches().get(block_id).is_some()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LeaderSettings<Ts, NodeId, NetworkAdapterSettings, BlendBroadcastSettings>
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
}

#[expect(clippy::allow_attributes_without_reason)]
pub struct CryptarchiaLeader<
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
    for CryptarchiaLeader<
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
    type Settings = LeaderSettings<
        TxS::Settings,
        NetAdapter::PeerId,
        NetAdapter::Settings,
        BlendService::BroadcastSettings,
    >;
    type State = overwatch::services::state::NoState<Self::Settings>;
    type StateOperator = overwatch::services::state::NoOperator<Self::State>;
    type Message = LeaderMsg;
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
    for CryptarchiaLeader<
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
        TryFrom<Block<ClPool::Item>> + TryInto<Block<ClPool::Item>>,
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
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self {
            service_resources_handle,
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

        let LeaderSettings {
            config: ledger_config,
            genesis_id,
            genesis_state,
            transaction_selector_settings,
            leader_config,
            network_adapter_settings,
            blend_broadcast_settings,
            bootstrap: bootstrap_config,
            ..
        } = self
            .service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        // TODO: check active slot coeff is exactly 1/30

        // These are blocks that have been pruned by the cryptarchia engine but have not
        // yet been deleted from the storage layer.
        let storage_blocks_to_remove = HashSet::new();
        let (cryptarchia, pruned_blocks, leader) = self
            .initialize_cryptarchia(
                genesis_id,
                &genesis_state,
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
            network_adapter,
            |cryptarchia, storage_blocks_to_remove, block| {
                let relays = &relays;
                let cryptarchia_clone = cryptarchia.clone();
                let storage_blocks_to_remove_clone = storage_blocks_to_remove.clone();
                async move {
                    Self::process_block_and_prune_storage(
                        cryptarchia,
                        block,
                        &storage_blocks_to_remove,
                        relays,
                    )
                    .await
                    .unwrap_or_else(|e| {
                        error!("Error processing block during IBD: {:?}", e);
                        (cryptarchia_clone, storage_blocks_to_remove_clone)
                    })
                }
            },
        );

        let (mut cryptarchia, mut storage_blocks_to_remove) = match initial_block_download
            .run(cryptarchia, storage_blocks_to_remove)
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

                error!("Initial Block Download did not complete successfully: {e}. Common causes: unresponsive initial peers, \
                network issues, or incorrect peer addresses. Consider retrying with different bootstrap peers.");

                return Err(DynError::from(format!(
                    "Initial Block Download failed: {e:?}"
                )));
            }
        };
        let async_loop = async {
            loop {
                tokio::select! {
                    Some(block) = incoming_blocks.next() => {
                        Self::log_received_block(&block);

                        if cryptarchia.has_block(&block.header().id()) {
                            info!(target: LOG_TARGET, "Block {:?} already processed, ignoring", block.header().id());
                            continue;
                        }

                        // Process the received block and update the cryptarchia state.
                        match Self::process_block_and_prune_storage(
                            cryptarchia.clone(),
                            block.clone(),
                            &storage_blocks_to_remove,
                            &relays,
                        ).await {
                            Ok((new_cryptarchia, new_storage_blocks_to_remove)) => {
                                cryptarchia = new_cryptarchia;
                                storage_blocks_to_remove = new_storage_blocks_to_remove;

                                info!(counter.consensus_processed_blocks = 1);
                            }
                            Err(Error::Ledger(nomos_ledger::LedgerError::ParentNotFound(parent))
                                | Error::Consensus(cryptarchia_engine::Error::ParentMissing(parent)),) => {
                                error!(target: LOG_TARGET, "Received block with parent {:?} that is not in the ledger state. Ignoring block.", parent);
                            }
                            Err(e) => {
                                error!(target: LOG_TARGET, "Error processing block: {:?}", e);
                            }
                        }
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
                                &relays
                            ).await;

                            if let Some(block) = block {
                                // apply our own block
                                match Self::process_block_and_prune_storage(
                                    cryptarchia.clone(),
                                    block.clone(),
                                    &storage_blocks_to_remove,
                                    &relays,
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

                    // Leader service doesn't handle external messages
                    Some(_msg) = self.service_resources_handle.inbound_relay.next() => {
                        // Ignore all messages - leader is driven by slot ticks only
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
    CryptarchiaLeader<
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
        TryFrom<Block<ClPool::Item>> + TryInto<Block<ClPool::Item>>,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
{
    async fn process_block_and_prune_storage(
        cryptarchia: Cryptarchia,
        block: Block<ClPool::Item>,
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
    ) -> Result<(Cryptarchia, HashSet<HeaderId>), Error> {
        let (cryptarchia, pruned_blocks) = Self::process_block(cryptarchia, block, relays).await?;

        let storage_blocks_to_remove = Self::delete_pruned_blocks_from_storage(
            pruned_blocks.stale_blocks().copied(),
            storage_blocks_to_remove,
            relays.storage_adapter(),
        )
        .await;

        Ok((cryptarchia, storage_blocks_to_remove))
    }

    /// Try to add a [`Block`] to [`Cryptarchia`].
    /// A [`Block`] is only added if it's valid
    #[expect(clippy::allow_attributes_without_reason)]
    #[instrument(level = "debug", skip(cryptarchia, relays))]
    async fn process_block(
        cryptarchia: Cryptarchia,
        block: Block<ClPool::Item>,
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
    ) -> Result<(Cryptarchia, PrunedBlocks<HeaderId>), Error> {
        debug!("received proposal {:?}", block);

        let sampled_blobs = match get_sampled_blobs(relays.sampling_relay().clone()).await {
            Ok(sampled_blobs) => sampled_blobs,
            Err(error) => {
                error!("Unable to retrieved sampled blobs: {error}");
                return Ok((cryptarchia, PrunedBlocks::new()));
            }
        };
        if !Self::validate_blocks_blobs(&block, &sampled_blobs) {
            error!("Invalid block: {block:?}");
            return Ok((cryptarchia, PrunedBlocks::new()));
        }

        // TODO: filter on time?
        let header = block.header();
        let id = header.id();
        let (cryptarchia, pruned_blocks) = cryptarchia.try_apply_header(header)?;

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

        // Leader service doesn't need to broadcast block notifications
        // Those are handled by the chain service

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
        let blobs_ids = get_sampled_blobs(relays.sampling_relay().clone());
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

    fn validate_blocks_blobs(
        block: &Block<ClPool::Item>,
        sampled_blobs_ids: &BTreeSet<da::BlobId>,
    ) -> bool {
        let validated_blobs = block
            .transactions()
            .flat_map(|tx| tx.mantle_tx().ops.iter())
            .filter_map(|op| {
                if let Op::ChannelBlob(op) = op {
                    Some(op.blob)
                } else {
                    None
                }
            })
            .all(|blob| sampled_blobs_ids.contains(&blob));
        validated_blobs
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
        genesis_state: &LedgerState,
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
        let lib_id = genesis_id; // Start from genesis for leader
        let state = choose_engine_state(
            lib_id,
            genesis_id,
            bootstrap_config,
            None, // No last engine state for leader
        );
        let mut cryptarchia =
            Cryptarchia::from_lib(lib_id, genesis_state.clone(), ledger_config, state);
        let leader = Leader::new(leader_config.utxos.clone(), leader_config.sk, ledger_config);

        let blocks = Self::get_blocks_in_range(lib_id, genesis_id, relays.storage_adapter()).await;

        // Skip LIB block since it's already applied
        let blocks = blocks.into_iter().skip(1);

        let mut pruned_blocks = PrunedBlocks::new();
        for block in blocks {
            match Self::process_block(cryptarchia.clone(), block, relays).await {
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
