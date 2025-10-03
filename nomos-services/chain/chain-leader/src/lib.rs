mod blend;
mod leadership;
mod relays;

use core::fmt::Debug;
use std::{collections::BTreeSet, fmt::Display, time::Duration};

use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use cryptarchia_engine::Slot;
use futures::{StreamExt as _, TryFutureExt as _};
pub use leadership::LeaderConfig;
use nomos_core::{
    block::Block,
    da,
    header::{Header, HeaderId},
    mantle::{AuthenticatedMantleTx, Op, Transaction, TxHash, TxSelect},
    proofs::leader_proof::{Groth16LeaderProof, LeaderPrivate},
};
use nomos_da_sampling::{
    DaSamplingService, DaSamplingServiceMsg, backend::DaSamplingServiceBackend,
};
use nomos_time::{SlotTick, TimeService, TimeServiceMessage};
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
    services::{AsServiceId, ServiceCore, ServiceData, relay::OutboundRelay},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use services_utils::wait_until_services_are_ready;
use thiserror::Error;
use tokio::sync::{broadcast, oneshot};
use tracing::{Level, debug, error, info, instrument, span};
use tracing_futures::Instrument as _;
use tx_service::{
    MempoolMsg, TxMempoolService, backend::RecoverableMempool,
    network::NetworkAdapter as MempoolAdapter,
};

use crate::{blend::BlendAdapter, leadership::Leader, relays::CryptarchiaConsensusRelays};

type SamplingRelay<BlobId> = OutboundRelay<DaSamplingServiceMsg<BlobId>>;

const LEADER_ID: &str = "Leader";

pub(crate) const LOG_TARGET: &str = "cryptarchia::leader";

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
    /// Request a new broadcast receiver that will yield all winning slots of
    /// the future epochs.
    ///
    /// The stream will yield items in one of two cases:
    /// * a new epoch starts -> winning slots for the new epoch
    /// * this service has just started mid-epoch -> winning slots for the
    ///   current epoch
    WinningPolEpochSlotStreamSubscribe {
        sender: oneshot::Sender<broadcast::Receiver<LeaderPrivate>>,
    },
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LeaderSettings<Ts, BlendBroadcastSettings> {
    #[serde(default)]
    pub transaction_selector_settings: Ts,
    pub config: nomos_ledger::Config,
    pub leader_config: LeaderConfig,
    pub blend_broadcast_settings: BlendBroadcastSettings,
}

#[expect(clippy::allow_attributes_without_reason)]
pub struct CryptarchiaLeader<
    BlendService,
    Mempool,
    MempoolNetAdapter,
    TxS,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    CryptarchiaService,
    RuntimeServiceId,
> where
    BlendService: nomos_blend_service::ServiceComponents,
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash>,
    Mempool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    Mempool::Settings: Clone,
    Mempool::Item: Clone + Eq + Debug + 'static,
    Mempool::Item: AuthenticatedMantleTx,
    MempoolNetAdapter:
        MempoolAdapter<RuntimeServiceId, Payload = Mempool::Item, Key = Mempool::Key>,
    TxS: TxSelect<Tx = Mempool::Item>,
    TxS::Settings: Send,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync + 'static,
    CryptarchiaService: CryptarchiaServiceData<Mempool::Item>,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    winning_pol_epoch_slots_sender: broadcast::Sender<LeaderPrivate>,
}

impl<
    BlendService,
    Mempool,
    MempoolNetAdapter,
    TxS,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    CryptarchiaService,
    RuntimeServiceId,
> ServiceData
    for CryptarchiaLeader<
        BlendService,
        Mempool,
        MempoolNetAdapter,
        TxS,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        CryptarchiaService,
        RuntimeServiceId,
    >
where
    BlendService: nomos_blend_service::ServiceComponents,
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash>,
    Mempool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    Mempool::Settings: Clone,
    Mempool::Item: AuthenticatedMantleTx + Clone + Eq + Debug,
    MempoolNetAdapter:
        MempoolAdapter<RuntimeServiceId, Payload = Mempool::Item, Key = Mempool::Key>,
    TxS: TxSelect<Tx = Mempool::Item>,
    TxS::Settings: Send,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync + 'static,
    CryptarchiaService: CryptarchiaServiceData<Mempool::Item>,
{
    type Settings = LeaderSettings<TxS::Settings, BlendService::BroadcastSettings>;
    type State = overwatch::services::state::NoState<Self::Settings>;
    type StateOperator = overwatch::services::state::NoOperator<Self::State>;
    type Message = LeaderMsg;
}

#[async_trait::async_trait]
impl<
    BlendService,
    Mempool,
    MempoolNetAdapter,
    TxS,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    CryptarchiaService,
    RuntimeServiceId,
> ServiceCore<RuntimeServiceId>
    for CryptarchiaLeader<
        BlendService,
        Mempool,
        MempoolNetAdapter,
        TxS,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        CryptarchiaService,
        RuntimeServiceId,
    >
where
    BlendService: ServiceData<
            Message = nomos_blend_service::message::ServiceMessage<BlendService::BroadcastSettings>,
        > + nomos_blend_service::ServiceComponents
        + Send
        + Sync
        + 'static,
    BlendService::BroadcastSettings: Clone + Send + Sync,
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash> + Send + Sync + 'static,
    Mempool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    Mempool::Settings: Clone + Send + Sync + 'static,
    Mempool::Item: Transaction<Hash = Mempool::Key>
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + Unpin
        + 'static,
    Mempool::Item: AuthenticatedMantleTx,
    MempoolNetAdapter: MempoolAdapter<RuntimeServiceId, Payload = Mempool::Item, Key = Mempool::Key>
        + Send
        + Sync
        + 'static,
    TxS: TxSelect<Tx = Mempool::Item> + Clone + Send + Sync + 'static,
    TxS::Settings: Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send + 'static,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + Send + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync + 'static,
    SamplingStorage:
        nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync + 'static,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync + 'static,
    CryptarchiaService: CryptarchiaServiceData<Mempool::Item>,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<Self>
        + AsServiceId<BlendService>
        + AsServiceId<
            TxMempoolService<
                MempoolNetAdapter,
                SamplingNetworkAdapter,
                SamplingStorage,
                Mempool,
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
        + AsServiceId<TimeService<TimeBackend, RuntimeServiceId>>
        + AsServiceId<CryptarchiaService>,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        let (winning_pol_epoch_slots_sender, _) = broadcast::channel(16);

        Ok(Self {
            service_resources_handle,
            winning_pol_epoch_slots_sender,
        })
    }

    #[expect(clippy::too_many_lines, reason = "TODO: Address this at some point.")]
    async fn run(mut self) -> Result<(), DynError> {
        let relays = CryptarchiaConsensusRelays::from_service_resources_handle::<
            Self,
            SamplingNetworkAdapter,
            SamplingStorage,
            TimeBackend,
            CryptarchiaService,
        >(&self.service_resources_handle)
        .await;

        // Create the API wrapper for chain service communication
        let cryptarchia_api = CryptarchiaServiceApi::<
            CryptarchiaService,
            Mempool::Item,
            RuntimeServiceId,
        >::new::<Self>(&self.service_resources_handle)
        .await?;

        let LeaderSettings {
            config: ledger_config,
            transaction_selector_settings,
            leader_config,
            blend_broadcast_settings,
        } = self
            .service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        // TODO: check active slot coeff is exactly 1/30

        let leader = Leader::new(leader_config.utxos.clone(), leader_config.sk, ledger_config);

        let tx_selector = TxS::new(transaction_selector_settings);

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
            BlendService,
            TxMempoolService<_, _, _, _, _>,
            DaSamplingService<_, _, _, _>,
            TimeService<_, _>,
            CryptarchiaService
        )
        .await?;

        let async_loop = async {
            loop {
                tokio::select! {
                    Some(SlotTick { slot, .. }) = slot_timer.next() => {
                        let chain_info = match cryptarchia_api.info().await {
                            Ok(info) => info,
                            Err(e) => {
                                error!("Failed to get chain info: {:?}", e);
                                continue;
                            }
                        };
                        let parent = chain_info.tip;

                        let tip_state = match cryptarchia_api.get_ledger_state(parent).await {
                            Ok(Some(state)) => state,
                            Ok(None) => {
                                error!("No ledger state found for tip {:?}", parent);
                                continue;
                            }
                            Err(e) => {
                                error!("Failed to get ledger state: {:?}", e);
                                continue;
                            }
                        };

                        let aged_tree = tip_state.aged_commitments();
                        let latest_tree = tip_state.latest_commitments();
                        debug!("ticking for slot {}", u64::from(slot));

                        let epoch_state = match cryptarchia_api.get_epoch_state(slot).await {
                            Ok(Some(state)) => state,
                            Ok(None) => {
                                error!("trying to propose a block for slot {} but epoch state is not available", u64::from(slot));
                                continue;
                            }
                            Err(e) => {
                                error!("Failed to get epoch state: {:?}", e);
                                continue;
                            }
                        };
                        if let Some(proof) = leader.build_proof_for(aged_tree, latest_tree, &epoch_state, slot).await {
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
                                // Process our own block first to ensure it's valid
                                match cryptarchia_api.process_leader_block(block.clone()).await {
                                    Ok(()) => {
                                        // Block successfully processed, now publish it to the network
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
                        handle_inbound_message(msg, &self.winning_pol_epoch_slots_sender);
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
        async_loop.instrument(span!(Level::TRACE, LEADER_ID)).await;

        Ok(())
    }
}

impl<
    BlendService,
    Mempool,
    MempoolNetAdapter,
    TxS,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    CryptarchiaService,
    RuntimeServiceId,
>
    CryptarchiaLeader<
        BlendService,
        Mempool,
        MempoolNetAdapter,
        TxS,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        TimeBackend,
        CryptarchiaService,
        RuntimeServiceId,
    >
where
    BlendService: ServiceData<
            Message = nomos_blend_service::message::ServiceMessage<BlendService::BroadcastSettings>,
        > + nomos_blend_service::ServiceComponents
        + Send
        + Sync
        + 'static,
    BlendService::BroadcastSettings: Send + Sync,
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash> + Send + Sync + 'static,
    Mempool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    Mempool::Settings: Clone + Send + Sync + 'static,
    Mempool::Item: Transaction<Hash = Mempool::Key>
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    Mempool::Item: AuthenticatedMantleTx,
    MempoolNetAdapter: MempoolAdapter<RuntimeServiceId, Payload = Mempool::Item, Key = Mempool::Key>
        + Send
        + Sync
        + 'static,
    TxS: TxSelect<Tx = Mempool::Item> + Clone + Send + Sync + 'static,
    TxS::Settings: Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = da::BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    CryptarchiaService: CryptarchiaServiceData<Mempool::Item>,
{
    #[expect(clippy::allow_attributes_without_reason)]
    #[instrument(level = "debug", skip(tx_selector, relays))]
    async fn propose_block(
        parent: HeaderId,
        slot: Slot,
        proof: Groth16LeaderProof,
        tx_selector: TxS,
        relays: &CryptarchiaConsensusRelays<
            BlendService,
            Mempool,
            MempoolNetAdapter,
            SamplingBackend,
            RuntimeServiceId,
        >,
    ) -> Option<Block<Mempool::Item>> {
        let mut output = None;
        let txs = get_mempool_contents(relays.mempool_relay().clone()).map_err(DynError::from);
        let sampling_relay = relays.sampling_relay().clone();
        let blobs_ids = get_sampled_blobs(sampling_relay);
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

fn handle_inbound_message(
    msg: LeaderMsg,
    winning_pol_epoch_slots_sender: &broadcast::Sender<LeaderPrivate>,
) {
    let LeaderMsg::WinningPolEpochSlotStreamSubscribe { sender } = msg;

    sender
        .send(winning_pol_epoch_slots_sender.subscribe())
        .unwrap_or_else(|_| {
            error!("Could not subscribe to POL epoch winning slots channel.");
        });
}
