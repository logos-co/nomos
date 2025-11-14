mod blob;
mod bootstrap;
mod mempool;
pub mod network;
mod relays;
mod states;
mod sync;

use core::fmt::Debug;
use std::{collections::HashMap, fmt::Display, hash::Hash, path::PathBuf, time::Duration};

use broadcast_service::{BlockBroadcastMsg, BlockBroadcastService, BlockInfo, SessionUpdate};
use chain_service::api::CryptarchiaServiceData;
pub use cryptarchia_engine::{Epoch, Slot};
use futures::{FutureExt as _, StreamExt as _};
use network::NetworkAdapter;
use nomos_core::{
    block::{Block, Proposal},
    da::{self},
    header::HeaderId,
    mantle::{
        AuthenticatedMantleTx, Transaction, TxHash,
        gas::MainnetGasConstants,
        genesis_tx::GenesisTx,
        ops::{Op, leader_claim::VoucherCm},
    },
    sdp::{ProviderId, ProviderInfo, ServiceType},
};
use nomos_da_sampling::{
    DaSamplingService, DaSamplingServiceMsg, backend::DaSamplingServiceBackend,
    mempool::DaMempoolAdapter,
};
pub use nomos_ledger::EpochState;
use nomos_ledger::LedgerState;
use nomos_network::NetworkService;
use nomos_time::TimeService;
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceCore, ServiceData, relay::OutboundRelay, state::StateUpdater},
};
use relays::BroadcastRelay;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use services_utils::{
    overwatch::{JsonFileBackend, RecoveryOperator, recovery::backends::FileBackendSettings},
    wait_until_services_are_ready,
};
use strum::IntoEnumIterator as _;
use thiserror::Error;
use tokio::time::Instant;
use tracing::{Level, debug, error, info, instrument, span, warn};
use tracing_futures::Instrument as _;
use tx_service::{
    TxMempoolService, backend::RecoverableMempool,
    network::NetworkAdapter as MempoolNetworkAdapter, storage::MempoolStorageAdapter,
};

use crate::{
    blob::{HistoricBlobStrategy, RecentBlobStrategy},
    bootstrap::{
        ibd::{self, InitialBlockDownload},
        state::choose_engine_state,
    },
    mempool::{MempoolAdapter as _, adapter::MempoolAdapter},
    relays::ChainNetworkRelays,
    states::CryptarchiaConsensusState,
    sync::orphan_handler::OrphanBlocksDownloader,
};
pub use crate::{
    bootstrap::config::{BootstrapConfig, IbdConfig, OfflineGracePeriodConfig},
    sync::config::{OrphanConfig, SyncConfig},
};

type SamplingRelay<BlobId> = OutboundRelay<DaSamplingServiceMsg<BlobId>>;

const CRYPTARCHIA_ID: &str = "Cryptarchia";

pub(crate) const LOG_TARGET: &str = "cryptarchia::service";

#[derive(Debug, Error)]
pub enum Error {
    #[error("Ledger error: {0}")]
    Ledger(#[from] nomos_ledger::LedgerError<HeaderId>),
    #[error("Consensus error: {0}")]
    Consensus(#[from] cryptarchia_engine::Error<HeaderId>),
    #[error("Serialization error: {0}")]
    Serialisation(#[from] nomos_core::codec::Error),
    #[error("Invalid block: {0}")]
    InvalidBlock(String),
    #[error("Mempool error: {0}")]
    Mempool(String),
    #[error("Blob validation failed: {0}")]
    BlobValidationFailed(#[from] blob::Error),
    #[error("Block header id not found: {0}")]
    HeaderIdNotFound(HeaderId),
    #[error("Service session not found: {0:?}")]
    ServiceSessionNotFound(ServiceType),
}

#[derive(Clone)]
struct Cryptarchia {
    ledger: nomos_ledger::Ledger<HeaderId>,
    consensus: cryptarchia_engine::Cryptarchia<HeaderId>,
    genesis_id: HeaderId,
}

impl Cryptarchia {
    /// Initialize a new [`Cryptarchia`] instance.
    pub fn from_lib(
        lib_id: HeaderId,
        lib_ledger_state: LedgerState,
        genesis_id: HeaderId,
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
            genesis_id,
        }
    }

    const fn tip(&self) -> HeaderId {
        self.consensus.tip()
    }

    const fn lib(&self) -> HeaderId {
        self.consensus.lib()
    }

    /// Create a new [`Cryptarchia`] with the updated state.
    #[must_use = "Returns a new instance with the updated state, without modifying the original."]
    fn try_apply_block<Tx>(&self, block: &Block<Tx>) -> Result<Self, Error>
    where
        Tx: AuthenticatedMantleTx,
    {
        let header = block.header();
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
            block.transactions(),
        )?;
        let (consensus, pruned_blocks) = self.consensus.receive_block(id, parent, slot)?;

        let mut cryptarchia = Self {
            ledger,
            consensus,
            genesis_id: self.genesis_id,
        };
        // Prune the ledger states of all the pruned blocks.
        cryptarchia.prune_ledger_states(pruned_blocks.all());

        Ok(cryptarchia)
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

    fn online(self) -> Self {
        let (consensus, _) = self.consensus.online();
        Self {
            ledger: self.ledger,
            consensus,
            genesis_id: self.genesis_id,
        }
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

    fn active_session_providers(
        &self,
        block_id: &HeaderId,
        service_type: ServiceType,
    ) -> Result<HashMap<ProviderId, ProviderInfo>, Error> {
        let ledger = self
            .ledger
            .state(block_id)
            .ok_or(Error::HeaderIdNotFound(*block_id))?;

        ledger
            .active_session_providers(service_type, self.ledger.config())
            .ok_or(Error::ServiceSessionNotFound(service_type))
    }

    fn active_sessions_numbers(
        &self,
        block_id: &HeaderId,
    ) -> Result<HashMap<ServiceType, u64>, Error> {
        let ledger = self
            .ledger
            .state(block_id)
            .ok_or(Error::HeaderIdNotFound(*block_id))?;

        Ok(ledger.active_sessions())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ChainNetworkSettings<NodeId, NetworkAdapterSettings>
where
    NodeId: Clone + Eq + Hash,
{
    pub config: nomos_ledger::Config,
    pub starting_state: StartingState,
    pub network_adapter_settings: NetworkAdapterSettings,
    pub recovery_file: PathBuf,
    pub bootstrap: BootstrapConfig<NodeId>,
    pub sync: SyncConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub enum StartingState {
    Genesis {
        genesis_tx: GenesisTx,
    },
    Lib {
        lib_id: HeaderId,
        lib_ledger_state: Box<LedgerState>,
        genesis_id: HeaderId,
    },
}

impl<NodeId, NetworkAdapterSettings> FileBackendSettings
    for ChainNetworkSettings<NodeId, NetworkAdapterSettings>
where
    NodeId: Clone + Eq + Hash,
{
    fn recovery_file(&self) -> &PathBuf {
        &self.recovery_file
    }
}

#[expect(clippy::allow_attributes_without_reason)]
pub struct ChainNetwork<
    Cryptarchia,
    NetAdapter,
    Mempool,
    MempoolNetAdapter,
    MempoolDaAdapter,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
> where
    Cryptarchia: CryptarchiaServiceData<Tx: Send + Sync>,
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Backend: 'static,
    NetAdapter::Settings: Send,
    NetAdapter::PeerId: Clone + Eq + Hash,
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash>,
    Mempool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    Mempool::Settings: Clone,
    Mempool::Storage: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
    Mempool::Item: Clone + Eq + Debug + 'static,
    Mempool::Item: AuthenticatedMantleTx,
    MempoolNetAdapter:
        MempoolNetworkAdapter<RuntimeServiceId, Payload = Mempool::Item, Key = Mempool::Key>,
    MempoolDaAdapter: DaMempoolAdapter,
    MempoolNetAdapter::Settings: Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    state: <Self as ServiceData>::State,
}

impl<
    Cryptarchia,
    NetAdapter,
    Mempool,
    MempoolNetAdapter,
    MempoolDaAdapter,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
> ServiceData
    for ChainNetwork<
        Cryptarchia,
        NetAdapter,
        Mempool,
        MempoolNetAdapter,
        MempoolDaAdapter,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        RuntimeServiceId,
    >
where
    Cryptarchia: CryptarchiaServiceData<Tx: Send + Sync>,
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Settings: Send,
    NetAdapter::PeerId: Clone + Eq + Hash,
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash>,
    Mempool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    Mempool::Settings: Clone,
    Mempool::Storage: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
    Mempool::Item: AuthenticatedMantleTx + Clone + Eq + Debug,
    MempoolNetAdapter:
        MempoolNetworkAdapter<RuntimeServiceId, Payload = Mempool::Item, Key = Mempool::Key>,
    MempoolDaAdapter: DaMempoolAdapter,
    MempoolNetAdapter::Settings: Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
{
    type Settings = ChainNetworkSettings<NetAdapter::PeerId, NetAdapter::Settings>;
    type State = CryptarchiaConsensusState<NetAdapter::PeerId, NetAdapter::Settings>;
    type StateOperator = RecoveryOperator<JsonFileBackend<Self::State, Self::Settings>>;
    type Message = ();
}

#[async_trait::async_trait]
impl<
    Cryptarchia,
    NetAdapter,
    Mempool,
    MempoolNetAdapter,
    MempoolDaAdapter,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
> ServiceCore<RuntimeServiceId>
    for ChainNetwork<
        Cryptarchia,
        NetAdapter,
        Mempool,
        MempoolNetAdapter,
        MempoolDaAdapter,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        RuntimeServiceId,
    >
where
    Cryptarchia: CryptarchiaServiceData<Tx: Send + Sync>,
    NetAdapter: NetworkAdapter<RuntimeServiceId, Block = Block<Mempool::Item>, Proposal = Proposal>
        + Clone
        + Send
        + Sync
        + 'static,
    NetAdapter::Settings: Send + Sync + 'static,
    NetAdapter::PeerId: Clone + Eq + Hash + Copy + Debug + Send + Sync + Unpin + 'static,
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash> + Send + Sync + 'static,
    Mempool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    Mempool::Settings: Clone + Send + Sync + 'static,
    Mempool::Storage: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
    Mempool::Item: Transaction<Hash = Mempool::Key>
        + AuthenticatedMantleTx
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + Unpin
        + 'static,
    MempoolNetAdapter: MempoolNetworkAdapter<RuntimeServiceId, Payload = Mempool::Item, Key = Mempool::Key>
        + Send
        + Sync
        + 'static,
    MempoolDaAdapter: DaMempoolAdapter + Send + Sync + 'static,
    MempoolNetAdapter::Settings: Send + Sync,
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
        + AsServiceId<Cryptarchia>
        + AsServiceId<NetworkService<NetAdapter::Backend, RuntimeServiceId>>
        + AsServiceId<BlockBroadcastService<RuntimeServiceId>>
        + AsServiceId<
            TxMempoolService<MempoolNetAdapter, Mempool, Mempool::Storage, RuntimeServiceId>,
        >
        + AsServiceId<
            DaSamplingService<
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                MempoolDaAdapter,
                RuntimeServiceId,
            >,
        >
        + AsServiceId<TimeService<TimeBackend, RuntimeServiceId>>,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        initial_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self {
            service_resources_handle,
            state: initial_state,
        })
    }

    #[expect(clippy::too_many_lines, reason = "TODO: Address this at some point.")]
    async fn run(mut self) -> Result<(), DynError> {
        let relays: ChainNetworkRelays<
            Cryptarchia,
            Mempool,
            MempoolNetAdapter,
            MempoolDaAdapter,
            NetAdapter,
            SamplingBackend,
            RuntimeServiceId,
        > = ChainNetworkRelays::from_service_resources_handle::<_, _, _>(
            &self.service_resources_handle,
        )
        .await;

        let ChainNetworkSettings {
            config: ledger_config,
            network_adapter_settings,
            bootstrap: bootstrap_config,
            sync: sync_config,
            ..
        } = self
            .service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        // TODO: check active slot coeff is exactly 1/30
        let cryptarchia = self
            .initialize_cryptarchia(&bootstrap_config, ledger_config.clone(), &relays)
            .await;

        let network_adapter =
            NetAdapter::new(network_adapter_settings, relays.network_relay().clone()).await;

        let mut incoming_proposals = network_adapter.proposals_stream().await?;
        let mut chainsync_events = network_adapter.chainsync_events_stream().await?;

        let mut orphan_downloader = Box::pin(OrphanBlocksDownloader::new(
            network_adapter.clone(),
            sync_config.orphan.max_orphan_cache_size,
        ));

        let recent_blob_validation = blob::Validation::<RecentBlobStrategy>::new(
            ledger_config.base_period_length(),
            relays.sampling_relay().clone(),
            relays.time_relay().clone(),
        );
        let historic_blob_validation = blob::Validation::<HistoricBlobStrategy>::new(
            ledger_config.base_period_length(),
            relays.sampling_relay().clone(),
            relays.time_relay().clone(),
        );

        wait_until_services_are_ready!(
            &self.service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            Cryptarchia,
            BlockBroadcastService<_>,
            NetworkService<_, _>,
            TxMempoolService<_, _, _, _>,
            DaSamplingService<_, _, _, _, _>,
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
            network_adapter,
            |cryptarchia, block| {
                let relays = &relays;
                let state_updater = &self.service_resources_handle.state_updater;
                let historic_blob_validation = &historic_blob_validation;
                async move {
                    Self::process_block_and_update_state(
                        cryptarchia,
                        block,
                        Some(historic_blob_validation),
                        relays,
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

        let mut cryptarchia = match initial_block_download.run().await {
            Ok(cryptarchia) => {
                info!("Initial Block Download completed successfully.");
                cryptarchia
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

        // Mark the service as ready if the chain is in the Online state.
        // If not, it will be marked as ready after Prolonged Bootstrap Period ends.
        if cryptarchia.state().is_online() {
            self.notify_service_ready();
        }

        let async_loop = async {
            loop {
                tokio::select! {
                    () = &mut prolonged_bootstrap_timer, if cryptarchia.is_boostrapping() => {
                        info!("Prolonged Bootstrap Period has passed. Switching to Online.");
                        cryptarchia = Self::switch_to_online(
                            cryptarchia,
                        );
                        Self::update_state(
                            &cryptarchia,
                            &self.service_resources_handle.state_updater,
                        );

                        self.notify_service_ready();
                    }

                    Some(proposal) = incoming_proposals.next() => {
                        self.handle_incoming_proposal(
                            proposal,
                            &mut cryptarchia,
                            &recent_blob_validation,
                            orphan_downloader.as_mut().get_mut(),
                            &relays,
                        )
                        .await;
                    }

                    Some(event) = chainsync_events.next() => {
                        // Forward the chain sync event to chain-service for handling
                        if let Err(e) = relays.cryptarchia().handle_chainsync_event(event).await {
                            error!(target: LOG_TARGET, "Failed to forward chainsync event to chain-service: {e}");
                        }
                    }

                    Some(block) = orphan_downloader.next(), if orphan_downloader.should_poll() => {
                        let header_id = block.header().id();
                        info!("Processing block from orphan downloader: {header_id:?}");

                        if cryptarchia.has_block(&block.header().id()) {
                            continue;
                        }

                        Self::log_received_block(&block);

                        match Self::process_block_and_update_state(
                            cryptarchia.clone(),
                            block.clone(),
                            Some(&historic_blob_validation),
                            &relays,
                            &self.service_resources_handle.state_updater
                        ).await {
                            Ok(new_cryptarchia) => {
                                cryptarchia = new_cryptarchia;

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
    Cryptarchia,
    NetAdapter,
    Mempool,
    MempoolNetAdapter,
    MempoolDaAdapter,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
>
    ChainNetwork<
        Cryptarchia,
        NetAdapter,
        Mempool,
        MempoolNetAdapter,
        MempoolDaAdapter,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        RuntimeServiceId,
    >
where
    Cryptarchia: CryptarchiaServiceData<Tx: Send + Sync>,
    NetAdapter: NetworkAdapter<RuntimeServiceId, Block = Block<Mempool::Item>, Proposal = Proposal>
        + Clone
        + Send
        + Sync
        + 'static,
    NetAdapter::Settings: Send + Sync + 'static,
    NetAdapter::PeerId: Clone + Eq + Hash + Copy + Debug + Send + Sync,
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash> + Send + Sync + 'static,
    Mempool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    Mempool::Settings: Clone + Send + Sync + 'static,
    Mempool::Storage: MempoolStorageAdapter<RuntimeServiceId> + Clone + Send + Sync,
    Mempool::Item: Transaction<Hash = Mempool::Key>
        + AuthenticatedMantleTx
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    MempoolNetAdapter: MempoolNetworkAdapter<RuntimeServiceId, Payload = Mempool::Item, Key = Mempool::Key>
        + Send
        + Sync
        + 'static,
    MempoolDaAdapter: DaMempoolAdapter + Send + Sync + 'static,
    MempoolNetAdapter::Settings: Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    RuntimeServiceId: Display + AsServiceId<Self>,
{
    fn notify_service_ready(&self) {
        self.service_resources_handle.status_updater.notify_ready();
        info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "This function does too much, need to deal with this at some point"
    )]
    async fn process_block_and_update_state<BlobStrategy>(
        cryptarchia: crate::Cryptarchia,
        block: Block<Mempool::Item>,
        blob_validation: Option<&blob::Validation<BlobStrategy>>,
        relays: &ChainNetworkRelays<
            Cryptarchia,
            Mempool,
            MempoolNetAdapter,
            MempoolDaAdapter,
            NetAdapter,
            SamplingBackend,
            RuntimeServiceId,
        >,
        state_updater: &StateUpdater<
            Option<CryptarchiaConsensusState<NetAdapter::PeerId, NetAdapter::Settings>>,
        >,
    ) -> Result<crate::Cryptarchia, Error>
    where
        BlobStrategy: blob::Strategy + Sync,
    {
        let cryptarchia = Self::process_block(cryptarchia, block, blob_validation, relays).await?;

        Self::update_state(&cryptarchia, state_updater);

        Ok(cryptarchia)
    }

    async fn handle_incoming_proposal(
        &self,
        proposal: Proposal,
        cryptarchia: &mut crate::Cryptarchia,
        recent_blob_validation: &blob::Validation<RecentBlobStrategy>,
        orphan_downloader: &mut OrphanBlocksDownloader<NetAdapter, RuntimeServiceId>,
        relays: &ChainNetworkRelays<
            Cryptarchia,
            Mempool,
            MempoolNetAdapter,
            MempoolDaAdapter,
            NetAdapter,
            SamplingBackend,
            RuntimeServiceId,
        >,
    ) where
        RuntimeServiceId: Send + Sync + 'static,
    {
        let block_id = proposal.header().id();

        if cryptarchia.has_block(&block_id) {
            info!(
                target: LOG_TARGET,
                "Block {:?} already processed, ignoring",
                block_id
            );
            return;
        }

        let block = match reconstruct_block_from_proposal(proposal, relays.mempool_adapter()).await
        {
            Ok(block) => block,
            Err(e) => {
                error!(
                    target: LOG_TARGET,
                    "Failed to reconstruct block from proposal: {:?}",
                    e
                );
                return;
            }
        };

        self.apply_reconstructed_block(
            block,
            cryptarchia,
            recent_blob_validation,
            orphan_downloader,
            relays,
        )
        .await;
    }

    fn handle_proposal_processing_error(
        err: Error,
        block_id: HeaderId,
        cryptarchia: &crate::Cryptarchia,
        orphan_downloader: &mut OrphanBlocksDownloader<NetAdapter, RuntimeServiceId>,
    ) where
        RuntimeServiceId: Send + Sync + 'static,
    {
        match err {
            Error::Ledger(nomos_ledger::LedgerError::ParentNotFound(parent))
            | Error::Consensus(cryptarchia_engine::Error::ParentMissing(parent)) => {
                orphan_downloader.enqueue_orphan(block_id, cryptarchia.tip(), cryptarchia.lib());

                error!(
                    target: LOG_TARGET,
                    "Received block with parent {:?} that is not in the ledger state. Ignoring block.",
                    parent
                );
            }
            other => {
                error!(
                    target: LOG_TARGET,
                    "Error processing reconstructed block: {:?}",
                    other
                );
            }
        }
    }

    async fn apply_reconstructed_block(
        &self,
        block: Block<Mempool::Item>,
        cryptarchia: &mut crate::Cryptarchia,
        recent_blob_validation: &blob::Validation<RecentBlobStrategy>,
        orphan_downloader: &mut OrphanBlocksDownloader<NetAdapter, RuntimeServiceId>,
        relays: &ChainNetworkRelays<
            Cryptarchia,
            Mempool,
            MempoolNetAdapter,
            MempoolDaAdapter,
            NetAdapter,
            SamplingBackend,
            RuntimeServiceId,
        >,
    ) where
        RuntimeServiceId: Send + Sync + 'static,
    {
        Self::log_received_block(&block);

        let block_id = block.header().id();

        match Self::process_block_and_update_state(
            cryptarchia.clone(),
            block,
            Some(recent_blob_validation),
            relays,
            &self.service_resources_handle.state_updater,
        )
        .await
        {
            Ok(new_cryptarchia) => {
                *cryptarchia = new_cryptarchia;
                orphan_downloader.remove_orphan(&block_id);
                info!(counter.consensus_processed_blocks = 1);
            }
            Err(err) => {
                Self::handle_proposal_processing_error(
                    err,
                    block_id,
                    cryptarchia,
                    orphan_downloader,
                );
            }
        }
    }

    fn update_state(
        cryptarchia: &crate::Cryptarchia,
        state_updater: &StateUpdater<
            Option<CryptarchiaConsensusState<NetAdapter::PeerId, NetAdapter::Settings>>,
        >,
    ) {
        match <Self as ServiceData>::State::from_cryptarchia_and_unpruned_blocks(cryptarchia) {
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
    async fn process_block<BlobStrategy>(
        cryptarchia: crate::Cryptarchia,
        block: Block<Mempool::Item>,
        blob_validation: Option<&blob::Validation<BlobStrategy>>,
        relays: &ChainNetworkRelays<
            Cryptarchia,
            Mempool,
            MempoolNetAdapter,
            MempoolDaAdapter,
            NetAdapter,
            SamplingBackend,
            RuntimeServiceId,
        >,
    ) -> Result<crate::Cryptarchia, Error>
    where
        BlobStrategy: blob::Strategy + Sync,
    {
        debug!("received proposal {:?}", block);

        // TODO: filter on time?
        let header = block.header();
        let id = header.id();
        let prev_lib = cryptarchia.lib();

        let previous_session_numbers = match cryptarchia.active_sessions_numbers(&prev_lib) {
            Ok(session_numbers) => session_numbers,
            Err(e) => {
                warn!("Error getting previous session numbers: {e}");
                ServiceType::iter().map(|s| (s, 0)).collect()
            }
        };

        if let Some(blob_validation) = blob_validation {
            blob_validation.validate(&block).await?;
        }

        let cryptarchia = cryptarchia.try_apply_block(&block)?;
        let new_lib = cryptarchia.lib();

        // remove included content from mempool
        relays
            .mempool_adapter()
            .mark_transactions_in_block(
                &block
                    .transactions()
                    .map(Transaction::hash)
                    .collect::<Vec<_>>(),
                id,
            )
            .await
            .unwrap_or_else(|e| error!("Could not mark transactions in block: {e}"));

        let blob_ids: Vec<da::BlobId> = block
            .transactions()
            .flat_map(|tx| tx.mantle_tx().ops.iter())
            .filter_map(|op| {
                if let Op::ChannelBlob(blob_op) = op {
                    Some(blob_op.blob)
                } else {
                    None
                }
            })
            .collect();

        if !blob_ids.is_empty() {
            mark_blob_in_block(relays.sampling_relay().clone(), blob_ids).await;
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

            Self::broadcast_session_updates_for_block(
                &cryptarchia,
                &new_lib,
                relays,
                Some(&previous_session_numbers),
            )
            .await;
        }

        Ok(cryptarchia)
    }

    fn log_received_block(block: &Block<Mempool::Item>) {
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

    /// Initialize cryptarchia
    /// It initialize cryptarchia from the LIB (initially genesis) +
    /// (optionally) known blocks which were received before the service
    /// restarted.
    ///
    /// # Arguments
    ///
    /// * `bootstrap_config` - The bootstrap configuration.
    /// * `ledger_config` - The ledger configuration.
    /// * `relays` - The relays object containing all the necessary relays for
    ///   the consensus.
    async fn initialize_cryptarchia(
        &self,
        bootstrap_config: &BootstrapConfig<NetAdapter::PeerId>,
        ledger_config: nomos_ledger::Config,
        relays: &ChainNetworkRelays<
            Cryptarchia,
            Mempool,
            MempoolNetAdapter,
            MempoolDaAdapter,
            NetAdapter,
            SamplingBackend,
            RuntimeServiceId,
        >,
    ) -> crate::Cryptarchia {
        let lib_id = self.state.lib;
        let genesis_id = self.state.genesis_id;
        let state = choose_engine_state(
            lib_id,
            genesis_id,
            bootstrap_config,
            self.state.last_engine_state.as_ref(),
        );
        let cryptarchia = crate::Cryptarchia::from_lib(
            lib_id,
            self.state.lib_ledger_state.clone(),
            genesis_id,
            ledger_config,
            state,
        );

        // Stream the already applied state.
        let init_tip = cryptarchia.tip();
        Self::broadcast_session_updates_for_block(&cryptarchia, &init_tip, relays, None).await;

        cryptarchia
    }

    fn switch_to_online(cryptarchia: crate::Cryptarchia) -> crate::Cryptarchia {
        cryptarchia.online()
    }

    async fn broadcast_session_updates_for_block(
        cryptarchia: &crate::Cryptarchia,
        block_id: &HeaderId,
        relays: &ChainNetworkRelays<
            Cryptarchia,
            Mempool,
            MempoolNetAdapter,
            MempoolDaAdapter,
            NetAdapter,
            SamplingBackend,
            RuntimeServiceId,
        >,
        previous_sessions: Option<&HashMap<ServiceType, u64>>,
    ) {
        let Ok(new_sessions) = cryptarchia.active_sessions_numbers(block_id) else {
            error!("Could not get active session numbers for block {block_id:?}");
            return;
        };

        for (service, new_session_number) in &new_sessions {
            Self::handle_service_update(
                cryptarchia,
                block_id,
                relays,
                previous_sessions,
                service,
                new_session_number,
            )
            .await;
        }
    }

    async fn handle_service_update(
        cryptarchia: &crate::Cryptarchia,
        block_id: &HeaderId,
        relays: &ChainNetworkRelays<
            Cryptarchia,
            Mempool,
            MempoolNetAdapter,
            MempoolDaAdapter,
            NetAdapter,
            SamplingBackend,
            RuntimeServiceId,
        >,
        previous_sessions: Option<&HashMap<ServiceType, u64>>,
        service: &ServiceType,
        new_session_number: &u64,
    ) {
        // If `previous_sessions` is provided, check if the session number has changed.
        // Otherwise, always broadcast (for initialization).
        if previous_sessions.is_some_and(|prev| {
            prev.get(service)
                .copied()
                .expect("previous session number is set")
                == *new_session_number
        }) {
            return;
        }

        match cryptarchia.active_session_providers(block_id, *service) {
            Ok(providers) => {
                let update = SessionUpdate {
                    session_number: *new_session_number,
                    providers,
                };

                let broadcast_relay = relays.broadcast_relay();

                let broadcast_future = match service {
                    ServiceType::BlendNetwork => {
                        broadcast_blend_session(broadcast_relay, update).boxed()
                    }
                    ServiceType::DataAvailability => {
                        broadcast_da_session(broadcast_relay, update).boxed()
                    }
                };

                if let Err(e) = broadcast_future.await {
                    error!("Failed to broadcast session update for {service:?}: {e}");
                }
            }
            Err(e) => {
                error!("Could not get session providers for service {service:?}: {e}");
            }
        }
    }
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

async fn broadcast_blend_session(
    broadcast_relay: &BroadcastRelay,
    session: SessionUpdate,
) -> Result<(), DynError> {
    broadcast_relay
        .send(BlockBroadcastMsg::BroadcastBlendSession(session))
        .await
        .map_err(|(error, _)| Box::new(error) as DynError)
}

async fn broadcast_da_session(
    broadcast_relay: &BroadcastRelay,
    session: SessionUpdate,
) -> Result<(), DynError> {
    broadcast_relay
        .send(BlockBroadcastMsg::BroadcastDASession(session))
        .await
        .map_err(|(error, _)| Box::new(error) as DynError)
}

/// Reconstruct a Block from a Proposal by looking up transactions from mempool
async fn reconstruct_block_from_proposal<Payload, Item>(
    proposal: Proposal,
    mempool: &MempoolAdapter<Payload, Item>,
) -> Result<Block<Item>, Error>
where
    Payload: Send + Sync,
    Item: AuthenticatedMantleTx<Hash = TxHash> + Clone + Send + Sync + 'static,
{
    let mempool_hashes: Vec<TxHash> = proposal.mempool_transactions().to_vec();
    let mempool_response = mempool
        .get_transactions_by_hashes(mempool_hashes)
        .await
        .map_err(|e| {
            Error::InvalidBlock(format!("Failed to get transactions from mempool: {e}"))
        })?;

    if !mempool_response.all_found() {
        return Err(Error::InvalidBlock(format!(
            "Failed to reconstruct block: {:?} mempool transactions not found",
            mempool_response.not_found()
        )));
    }

    // TODO: recover service reward
    let service_reward = None;

    let reconstructed_transactions = mempool_response.into_found();

    let header = proposal.header().clone();
    let signature = *proposal.signature();

    let block = Block::reconstruct(
        header,
        reconstructed_transactions,
        service_reward,
        signature,
    )
    .map_err(|e| Error::InvalidBlock(format!("Invalid block: {e}")))?;

    Ok(block)
}
