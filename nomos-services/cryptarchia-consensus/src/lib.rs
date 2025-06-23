pub mod blend;
mod leadership;
mod messages;
pub mod network;
mod relays;
mod states;
pub mod storage;

use core::fmt::Debug;
use std::{
    collections::{BTreeSet, HashSet},
    fmt::Display,
    hash::Hash,
    path::PathBuf,
    time::Duration,
};

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
use nomos_network::{message::ChainSyncEvent, NetworkService};
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend, StorageService};
use nomos_time::{SlotTick, TimeService, TimeServiceMessage};
use overwatch::{
    services::{relay::OutboundRelay, state::StateUpdater, AsServiceId, ServiceCore, ServiceData},
    DynError, OpaqueServiceResourcesHandle,
};
use rand::{RngCore, SeedableRng};
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
    relays::CryptarchiaConsensusRelays,
    states::CryptarchiaConsensusState,
    storage::{adapters::StorageAdapter, StorageAdapter as _},
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

    /// Prune the in-memory storage of forks older than or equal to the Last
    /// Immutable Block (LIB).
    ///
    /// The old blocks are removed from the consensus engine and their state
    /// removed from the ledger state.
    pub fn prune_old_forks(&mut self) -> impl Iterator<Item = HeaderId> {
        // We need to iterate once, then return the unconsumed iterator, so we need to
        // collect first.
        let lib_depth = self
            .consensus
            .tip_branch()
            .length()
            .checked_sub(self.consensus.lib_branch().length())
            .expect("Canonical chain tip heigh must be higher than LIB height.");
        let old_forks_pruned = self.consensus.prune_forks(lib_depth).collect::<Vec<_>>();
        let mut pruned_states_count = 0usize;
        for pruned_block in &old_forks_pruned {
            if self.ledger.prune_state_at(pruned_block) {
                pruned_states_count = pruned_states_count.saturating_add(1);
            } else {
                tracing::error!(
                   target: LOG_TARGET,
                    "Failed to prune ledger state for block {:?} which should exist.",
                    pruned_block
                );
            }
        }
        tracing::debug!(target: LOG_TARGET, "Pruned {pruned_states_count} old forks and their ledger states.");

        old_forks_pruned.into_iter()
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

        // These are blocks that have been pruned by the cryptarchia engine but have not
        // yet been deleted from the storage layer.
        let mut previously_pruned_blocks = self.initial_state.prunable_blocks.clone();
        let (mut cryptarchia, mut leader) = Self::initialize_cryptarchia(
            self.initial_state,
            ledger_config,
            leader_config,
            &relays,
            &self.block_subscription_sender,
        )
        .await;

        let network_adapter =
            NetAdapter::new(network_adapter_settings, relays.network_relay().clone()).await;
        let tx_selector = TxS::new(transaction_selector_settings);
        let blob_selector = BS::new(blob_selector_settings);

        let mut incoming_blocks = network_adapter.blocks_stream().await?;
        let mut chainsync_events = network_adapter.chainsync_events_stream().await?;

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
            BlendService<_, _, _>,
            TxMempoolService<_, _, _>,
            DaMempoolService<_, _, _, _, _, _, _, _, _, _, _>,
            DaSamplingService<_, _, _, _, _, _, _, _, _>,
            StorageService<_, _>,
            TimeService<_, _>
        )
        .await?;

        let async_loop = async {
            loop {
                tokio::select! {
                    Some(block) = incoming_blocks.next() => {
                        Self::log_received_block(&block);

                        // Process the received block and update the cryptarchia state.
                        cryptarchia = Self::process_block_and_update_state(
                            cryptarchia,
                            &mut leader,
                            block.clone(),

                            &mut previously_pruned_blocks,
                            &relays,
                            &self.block_subscription_sender,
                            self.service_resources_handle
                                        .state_updater.clone()
                        ).await;

                        info!(counter.consensus_processed_blocks = 1);
                    }

                    Some(SlotTick { slot, .. }) = slot_timer.next() => {
                        let parent = cryptarchia.tip();
                        let note_tree = cryptarchia.tip_state().lead_commitments();
                        debug!("ticking for slot {}", u64::from(slot));

                        let Some(epoch_state) = cryptarchia.epoch_state_for_slot(slot) else {
                            error!("trying to propose a block for slot {} but epoch state is not available", u64::from(slot));
                            continue;
                        };
                        if let Some(proof) = leader.build_proof_for(note_tree, epoch_state, slot, parent).await {
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
                                cryptarchia = Self::process_block_and_update_state(
                                     cryptarchia,
                                    &mut leader,
                                    block.clone(),

                                    &mut previously_pruned_blocks,
                                    &relays,
                                    &self.block_subscription_sender,
                                    self.service_resources_handle
                                        .state_updater.clone()
                                        ,
                                )
                                .await;
                                blend_adapter.blend(block).await;
                            }
                        }
                    }

                    Some(msg) = self.service_resources_handle.inbound_relay.next() => {
                        Self::process_message(&cryptarchia, &self.block_subscription_sender, msg);
                    }

                    Some(event) = chainsync_events.next() => {
                        if let ChainSyncEvent::ProvideTipRequest{reply_sender} = event {
                            let tip = cryptarchia.tip();
                            if let Err(e) = reply_sender.send(tip).await {
                                error!("Failed to send tip header: {e}");
                            }
                        }
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

    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusState and CryptarchiaConsensusRelays amount of generics."
    )]
    #[expect(
        clippy::future_not_send,
        reason = "Internal function, settings are not Sync"
    )]
    async fn process_block_and_update_state(
        mut cryptarchia: Cryptarchia<Online>,
        leader: &mut Leader,
        block: Block<ClPool::Item, DaPool::Item>,
        previously_pruned_blocks: &mut HashSet<HeaderId>,
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
        block_subscription_sender: &broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
        state_updater: StateUpdater<
            Option<
                CryptarchiaConsensusState<
                    TxS::Settings,
                    BS::Settings,
                    NetAdapter::Settings,
                    BlendAdapter::Settings,
                >,
            >,
        >,
    ) -> Cryptarchia<Online> {
        cryptarchia = Self::process_block(
            cryptarchia,
            leader,
            block,
            relays,
            block_subscription_sender,
        )
        .await;

        // This will modify `previously_pruned_blocks` to include blocks which are not
        // tracked by Cryptarchia anymore but have not been deleted from the persistence
        // layer.
        Self::prune_forks(
            &mut cryptarchia,
            leader,
            relays.storage_adapter(),
            previously_pruned_blocks,
        )
        .await;

        match <Self as ServiceData>::State::from_cryptarchia_and_unpruned_blocks(
            &cryptarchia,
            leader,
            previously_pruned_blocks.clone(),
        ) {
            Ok(state) => {
                state_updater.update(Some(state));
            }
            Err(e) => {
                error!(target: LOG_TARGET, "Failed to update state: {}", e);
            }
        }

        cryptarchia
    }

    /// Try to add a [`Block`] to [`Cryptarchia`].
    /// A [`Block`] is only added if it's valid
    #[expect(clippy::allow_attributes_without_reason)]
    #[expect(clippy::type_complexity)]
    #[instrument(level = "debug", skip(cryptarchia, leader, relays))]
    async fn process_block<State: CryptarchiaState>(
        mut cryptarchia: Cryptarchia<State>,
        leader: &mut Leader,
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
        block_broadcaster: &broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    ) -> Cryptarchia<State> {
        debug!("received proposal {:?}", block);

        let sampled_blobs = match get_sampled_blobs(relays.sampling_relay().clone()).await {
            Ok(sampled_blobs) => sampled_blobs,
            Err(error) => {
                error!("Unable to retrieved sampled blobs: {error}");
                return cryptarchia;
            }
        };
        if !Self::validate_blocks_blobs(&block, &sampled_blobs) {
            error!("Invalid block: {block:?}");
            return cryptarchia;
        }

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
                    error!("Could not notify block to services {e}");
                }

                cryptarchia = new_state;
            }
            Err(
                Error::Ledger(nomos_ledger::LedgerError::ParentNotFound(parent))
                | Error::Consensus(cryptarchia_engine::Error::ParentMissing(parent)),
            ) => {
                debug!("missing parent {:?}", parent);
                // TODO: request parent block
            }
            Err(e) => debug!("invalid block {:?}: {e:?}", block),
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
    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusState and CryptarchiaConsensusRelays amount of generics."
    )]
    async fn initialize_cryptarchia(
        initial_state: CryptarchiaConsensusState<
            TxS::Settings,
            BS::Settings,
            NetAdapter::Settings,
            BlendAdapter::Settings,
        >,
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
        block_subscription_sender: &broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    ) -> (Cryptarchia<Online>, Leader) {
        let lib_id = initial_state.lib;
        let mut cryptarchia =
            <Cryptarchia<Online>>::from_lib(lib_id, initial_state.lib_ledger_state, ledger_config);
        let mut leader = Leader::new(
            lib_id,
            initial_state.lib_leader_notes.clone(),
            leader_config.nf_sk,
            ledger_config,
        );

        let blocks =
            Self::get_blocks_in_range(lib_id, initial_state.tip, relays.storage_adapter()).await;

        // Skip LIB block since it's already applied
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

    /// Remove the in-memory storage of stale blocks from the cryptarchia engine
    /// and of stale `PoL` notes from the `PoL` machinery.
    /// Furthermore, it attempts to remove the deleted block data from storage
    /// as well, also including the blocks provided in `prunable_blocks` that
    /// might belong to previous pruning operations and that failed for some
    /// reason.
    ///
    /// Any block that fails to be deleted from the storage layer is added to
    /// the provided `prunable_blocks` parameter and will be picked up at the
    /// next invocation of this function.
    async fn prune_forks(
        cryptarchia: &mut Cryptarchia<Online>,
        leader: &mut Leader,
        storage_adapter: &StorageAdapter<Storage, TxS::Tx, BS::BlobId, RuntimeServiceId>,
        prunable_blocks: &mut HashSet<HeaderId>,
    ) {
        let old_forks_pruned = cryptarchia.prune_old_forks().collect::<HashSet<_>>();
        // And remove the notes of the pruned blocks from the leader state.
        for old_fork_pruned in &old_forks_pruned {
            if leader.prune_notes_at(old_fork_pruned) {
                tracing::debug!(
                    target: LOG_TARGET,
                    "Leader proof for block {old_fork_pruned} removed from in-memory storage."
                );
            }
        }

        // We try to delete both freshly pruned forks as well as old pruned blocks.
        match Self::delete_pruned_blocks_from_storage(
            old_forks_pruned
                .iter()
                .chain(prunable_blocks.iter())
                .copied(),
            storage_adapter,
        )
        .await
        {
            // All blocks, past and present, have been successfully deleted from storage.
            Ok(()) => *prunable_blocks = HashSet::new(),
            // We retain the blocks that failed to be deleted.
            Err(failed_blocks) => {
                *prunable_blocks = failed_blocks
                    .into_iter()
                    .map(|(block_id, _)| block_id)
                    .collect();
            }
        }
    }

    /// Send a bulk deletion request to the storage adapter.
    ///
    /// If no request fails, the method returns `Ok()`.
    /// If any request fails, the header ID and the generated error for each
    /// failing request are collected and returned as part of the `Err`
    /// result.
    async fn delete_pruned_blocks_from_storage<Headers>(
        pruned_block_headers: Headers,
        storage_adapter: &StorageAdapter<Storage, TxS::Tx, BS::BlobId, RuntimeServiceId>,
    ) -> Result<(), Vec<(HeaderId, DynError)>>
    where
        Headers: Iterator<Item = HeaderId> + Send,
    {
        let blocks_to_delete = pruned_block_headers.collect::<Vec<_>>();
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
