use std::collections::HashSet;

use async_trait::async_trait;
use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use chain_service::storage::{adapters::storage::StorageAdapter, StorageAdapter as _};
use chain_service::LibUpdate;
use nomos_core::{
    block::Block,
    header::HeaderId,
    mantle::{keys::PublicKey, SignedMantleTx, Utxo, Value},
};
use nomos_storage::backends::StorageBackend;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use tokio::sync::oneshot;
use tracing::{debug, error, info, trace};
use wallet::{Wallet, WalletBlock, WalletError};

#[derive(Debug, thiserror::Error)]
pub enum WalletServiceError {
    #[error("Ledger state corresponding to block {0} not found")]
    LedgerStateNotFound(HeaderId),

    #[error("Wallet state corresponding to block {0} not found")]
    WalletStateNotFound(HeaderId),

    #[error("Failed to apply historical block {block_id} to wallet: {source}")]
    HistoricalBlockApplicationFailed {
        block_id: HeaderId,
        #[source]
        source: WalletError,
    },

    #[error("Block {block_id} not found in storage during wallet sync")]
    BlockNotFoundInStorage { block_id: HeaderId },

    #[error("Cryptarchia API error: {0}")]
    CryptarchiaApi(#[from] DynError),
}

#[derive(Debug)]
pub enum WalletMsg {
    GetBalance {
        tip: HeaderId,
        pk: PublicKey,
        tx: oneshot::Sender<Result<Option<Value>, WalletError>>,
    },
    GetUtxosForAmount {
        tip: HeaderId,
        amount: Value,
        pks: Vec<PublicKey>,
        tx: oneshot::Sender<Result<Option<Vec<Utxo>>, WalletError>>,
    },
    GetLeaderAgedNotes {
        tip: HeaderId,
        tx: oneshot::Sender<Result<Vec<Utxo>, WalletServiceError>>,
    },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WalletServiceSettings {
    pub known_keys: HashSet<PublicKey>,
}

pub struct WalletService<Cryptarchia, Storage, RuntimeServiceId> {
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _storage: std::marker::PhantomData<Storage>,
}

impl<Cryptarchia, Storage, RuntimeServiceId> ServiceData
    for WalletService<Cryptarchia, Storage, RuntimeServiceId>
{
    type Settings = WalletServiceSettings;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = WalletMsg;
}

#[async_trait]
impl<Cryptarchia, Storage, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for WalletService<Cryptarchia, Storage, RuntimeServiceId>
where
    Cryptarchia: CryptarchiaServiceData,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as nomos_storage::api::chain::StorageChainApi>::Block:
        TryFrom<Block<SignedMantleTx>> + TryInto<Block<SignedMantleTx>>,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<Cryptarchia>
        + AsServiceId<nomos_storage::StorageService<Storage, RuntimeServiceId>>
        + std::fmt::Debug
        + std::fmt::Display
        + Send
        + Sync
        + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self {
            service_resources_handle,
            _storage: std::marker::PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), DynError> {
        let Self {
            mut service_resources_handle,
            ..
        } = self;

        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let storage_relay = service_resources_handle
            .overwatch_handle
            .relay::<nomos_storage::StorageService<Storage, RuntimeServiceId>>()
            .await?;

        // Create the API wrapper for cleaner communication
        let cryptarchia_api = CryptarchiaServiceApi::<Cryptarchia, RuntimeServiceId>::new::<Self>(
            &service_resources_handle,
        )
        .await?;

        // Create StorageAdapter for cleaner block operations
        let storage_adapter =
            StorageAdapter::<Storage, SignedMantleTx, RuntimeServiceId>::new(storage_relay).await;

        // Query chain service for current state using the API
        let chain_info = cryptarchia_api.info().await?;

        info!(
            tip = ?chain_info.tip,
            lib = ?chain_info.lib,
            slot = ?chain_info.slot,
            "Wallet connecting to chain"
        );

        // IMPORTANT: subscribe for new blocks *before* we sync from lib.
        // The blocks subscription only notifies us of new blocks that were observed
        // after we subscribed.
        //
        // If syncing from LIB takes some time, there may be a gap between the blocks
        // we observed when we first bootstrap from LIB and from when we start
        // subscribing to new blocks.
        //
        // LIB <- B_1 <- B_2 <- .. <- B_tip <- B_tip+1
        // --------------------------------
        //    synced during bootstrapping
        //
        // B_tip+1 may have been produced while we were bootstrapping. If we subscribe
        // to new blocks before we start bootstrapping, B_tip+1 and later blocks
        // will remain in the channel buffered until we are finished processing
        // up to B_tip.

        // Subscribe to block updates using the API
        let mut new_block_receiver = cryptarchia_api.subscribe_new_blocks().await?;

        // Subscribe to LIB updates for wallet state pruning
        let mut lib_receiver = cryptarchia_api.subscribe_lib_updates().await?;

        // Initialize wallet from LIB and LIB LedgerState
        let lib = chain_info.lib;

        // Fetch the ledger state at LIB using the API
        let lib_ledger = cryptarchia_api
            .get_ledger_state(lib)
            .await?
            .ok_or(WalletServiceError::LedgerStateNotFound(lib))?;

        let mut wallet = Wallet::from_lib(settings.known_keys.clone(), lib, &lib_ledger);

        // Request headers from LIB to tip to sync wallet state using the API
        let headers = cryptarchia_api
            .get_headers(Some(lib), Some(chain_info.tip))
            .await?;

        debug!(
            header_count = headers.len(),
            "Received headers from LIB to tip"
        );

        // Fetch and apply blocks for each header to sync wallet state
        for header_id in headers {
            if header_id == lib {
                // We already have state at LIB by bootstrapping from LedgerState
                continue;
            }
            if let Some(block) = storage_adapter.get_block(&header_id).await {
                let wallet_block = WalletBlock::from(block);
                match wallet.apply_block(&wallet_block) {
                    Ok(()) => {
                        trace!(block_id = ?header_id, "Applied historical block to wallet");
                    }
                    Err(e) => {
                        let service_error = WalletServiceError::HistoricalBlockApplicationFailed {
                            block_id: header_id,
                            source: e,
                        };
                        error!(error = %service_error, "Failed to apply historical block to wallet");
                        return Err(service_error.into());
                    }
                }
            } else {
                let service_error = WalletServiceError::BlockNotFoundInStorage {
                    block_id: header_id,
                };
                error!(error = %service_error, "Block not found in storage during wallet sync");
                return Err(service_error.into());
            }
        }

        service_resources_handle.status_updater.notify_ready();
        info!("Wallet service is ready and subscribed to blocks");

        loop {
            tokio::select! {
                Some(msg) = service_resources_handle.inbound_relay.recv() => {
                    Self::handle_wallet_message(msg, &wallet, &cryptarchia_api).await;
                }
                Ok(header_id) = new_block_receiver.recv() => {
                    let Some(block) = storage_adapter.get_block(&header_id).await else {
                        panic!("missing block in storage");
                    };
                    let wallet_block = WalletBlock::from(block);
                    match wallet.apply_block(&wallet_block) {
                        Ok(()) => {
                            trace!(block_id = ?wallet_block.id, "Applied block to wallet");
                        }
                        Err(e) => {
                            error!(error = ?e, "Failed to apply block to wallet");
                        }
                    }
                }
                Ok(lib_update) = lib_receiver.recv() => {
                    Self::handle_lib_update(&lib_update, &mut wallet);
                }
            }
        }
    }
}

impl<Cryptarchia, Storage, RuntimeServiceId> WalletService<Cryptarchia, Storage, RuntimeServiceId>
where
    Cryptarchia: CryptarchiaServiceData + Send + 'static,
    Storage: StorageBackend + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Cryptarchia> + std::fmt::Debug + std::fmt::Display + Sync,
{
    async fn handle_wallet_message(
        msg: WalletMsg,
        wallet: &Wallet,
        cryptarchia_api: &CryptarchiaServiceApi<Cryptarchia, RuntimeServiceId>,
    ) {
        match msg {
            WalletMsg::GetBalance { tip, pk, tx } => {
                let balance = wallet.balance(tip, pk);
                if tx.send(balance).is_err() {
                    error!("Failed to respond to GetBalance");
                }
            }
            WalletMsg::GetUtxosForAmount {
                tip,
                amount,
                pks,
                tx,
            } => {
                let utxos = wallet.utxos_for_amount(tip, amount, pks);
                if tx.send(utxos).is_err() {
                    error!("Failed to respond to GetUtxosForAmount");
                }
            }
            WalletMsg::GetLeaderAgedNotes { tip, tx } => {
                Self::get_leader_aged_notes(tip, tx, wallet, cryptarchia_api).await;
            }
        }
    }

    async fn get_leader_aged_notes(
        tip: HeaderId,
        tx: oneshot::Sender<Result<Vec<Utxo>, WalletServiceError>>,
        wallet: &Wallet,
        cryptarchia_api: &CryptarchiaServiceApi<Cryptarchia, RuntimeServiceId>,
    ) {
        // Get the ledger state at the specified tip
        let Ok(Some(ledger_state)) = cryptarchia_api.get_ledger_state(tip).await else {
            if tx
                .send(Err(WalletServiceError::LedgerStateNotFound(tip)))
                .is_err()
            {
                error!("Failed to respond to GetLeaderAgedNotes");
            }
            return;
        };

        // TODO: there may be a race condition here where the caller knows a more recent
        // tip than the wallet. In that case, we will have received a
        // LedgerState for the tip from Cryptarchia, but we would be missing the
        // WalletState for that tip.
        //
        // Currently the way we deal with that is to just return an error but that's
        // not ideal.
        //
        // To resolve, we could trigger an immediate sync here to ensure that the
        // wallet is in sync with the caller and Cryptarchia.
        let Ok(wallet_state) = wallet.wallet_state_at(tip) else {
            if tx
                .send(Err(WalletServiceError::WalletStateNotFound(tip)))
                .is_err()
            {
                error!("Failed to respond to GetLeaderAgedNotes");
            }
            return;
        };

        let aged_utxos = ledger_state.epoch_state().utxos.utxos();
        let eligible_utxos: Vec<Utxo> = wallet_state
            .utxos
            .iter()
            .filter(|(note_id, _)| aged_utxos.contains_key(note_id))
            .map(|(_, utxo)| *utxo)
            .collect();

        if tx.send(Ok(eligible_utxos)).is_err() {
            error!("Failed to respond to GetLeaderAgedNotes");
        }
    }

    fn handle_lib_update(lib_update: &LibUpdate, wallet: &mut Wallet) {
        debug!(
            new_lib = ?lib_update.new_lib,
            stale_blocks_count = lib_update.pruned_blocks.stale_blocks.len(),
            immutable_blocks_count = lib_update.pruned_blocks.immutable_blocks.len(),
            "Received LIB update"
        );

        wallet.prune_states(lib_update.pruned_blocks.all());
    }
}
