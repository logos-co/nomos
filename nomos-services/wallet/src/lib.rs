use std::{collections::HashSet, time::Duration};

use async_trait::async_trait;
use chain_service::{
    LibUpdate,
    api::{CryptarchiaServiceApi, CryptarchiaServiceData},
    storage::{StorageAdapter as _, adapters::storage::StorageAdapter},
};
use nomos_core::{
    block::Block,
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, Utxo, Value, keys::PublicKey},
};
use nomos_storage::backends::StorageBackend;
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        state::{NoOperator, NoState},
    },
};
use serde::{Serialize, de::DeserializeOwned};
use services_utils::wait_until_services_are_ready;
use tokio::sync::oneshot;
use tracing::{debug, error, info, trace};
use wallet::{Wallet, WalletBlock, WalletError};

#[derive(Debug, thiserror::Error)]
pub enum WalletServiceError {
    #[error("Ledger state corresponding to block {0} not found")]
    LedgerStateNotFound(HeaderId),

    #[error("Wallet state corresponding to block {0} not found")]
    WalletStateNotFound(HeaderId),

    #[error("Failed to apply historical block {0} to wallet")]
    BackfillFailedToApplyBlock(HeaderId),

    #[error("Block {0} not found in storage during wallet sync")]
    BlockNotFoundInStorage(HeaderId),

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

pub struct WalletService<Cryptarchia, Tx, Storage, RuntimeServiceId> {
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _storage: std::marker::PhantomData<Storage>,
    _tx: std::marker::PhantomData<Tx>,
}

impl<Cryptarchia, Tx, Storage, RuntimeServiceId> ServiceData
    for WalletService<Cryptarchia, Tx, Storage, RuntimeServiceId>
{
    type Settings = WalletServiceSettings;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = WalletMsg;
}

#[async_trait]
impl<Cryptarchia, Tx, Storage, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for WalletService<Cryptarchia, Tx, Storage, RuntimeServiceId>
where
    Tx: AuthenticatedMantleTx + Send + Sync + Clone + Eq + Serialize + DeserializeOwned + 'static,
    Cryptarchia: CryptarchiaServiceData<Tx = Tx>,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as nomos_storage::api::chain::StorageChainApi>::Block:
        TryFrom<Block<Tx>> + TryInto<Block<Tx>>,
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
            _tx: std::marker::PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), DynError> {
        let Self {
            mut service_resources_handle,
            ..
        } = self;

        wait_until_services_are_ready!(
            &service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            nomos_storage::StorageService<_, _>,
            Cryptarchia
        )
        .await?;

        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let storage_relay = service_resources_handle
            .overwatch_handle
            .relay::<nomos_storage::StorageService<Storage, RuntimeServiceId>>()
            .await?;

        // Create the API wrapper for cleaner communication
        let cryptarchia_api = CryptarchiaServiceApi::<Cryptarchia, RuntimeServiceId>::new(
            &service_resources_handle.overwatch_handle,
        )
        .await?;

        // Create StorageAdapter for cleaner block operations
        let storage_adapter =
            StorageAdapter::<Storage, Tx, RuntimeServiceId>::new(storage_relay).await;

        // Query chain service for current state using the API
        let chain_info = cryptarchia_api.info().await?;

        info!(
            tip = ?chain_info.tip,
            lib = ?chain_info.lib,
            slot = ?chain_info.slot,
            "Wallet connecting to chain"
        );

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

        Self::backfill_missing_blocks(
            chain_info.tip,
            &mut wallet,
            &storage_adapter,
            &cryptarchia_api,
        )
        .await?;

        service_resources_handle.status_updater.notify_ready();
        info!("Wallet service is ready and subscribed to blocks");

        loop {
            tokio::select! {
                Some(msg) = service_resources_handle.inbound_relay.recv() => {
                    Self::handle_wallet_message(msg, &mut wallet, &storage_adapter, &cryptarchia_api).await;
                }
                Ok(header_id) = new_block_receiver.recv() => {
                    let Some(block) = storage_adapter.get_block(&header_id).await else {
                        error!(block_id=?header_id, "Missing block in storage");
                        continue;
                    };
                    let wallet_block = WalletBlock::from(block);
                    match wallet.apply_block(&wallet_block) {
                        Ok(()) => {
                            trace!(block_id = ?wallet_block.id, "Applied block to wallet");
                        }
                        Err(WalletError::UnknownBlock) => {

                            info!(block_id = ?header_id, "Missing block in wallet, backfilling");
                            Self::backfill_missing_blocks(wallet_block.id, &mut wallet, &storage_adapter, &cryptarchia_api).await?;
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

impl<Cryptarchia, Tx, Storage, RuntimeServiceId>
    WalletService<Cryptarchia, Tx, Storage, RuntimeServiceId>
where
    Tx: AuthenticatedMantleTx + Send + Sync + Clone + Eq + Serialize + DeserializeOwned + 'static,
    Cryptarchia: CryptarchiaServiceData<Tx = Tx> + Send + 'static,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as nomos_storage::api::chain::StorageChainApi>::Block:
        TryFrom<Block<Tx>> + TryInto<Block<Tx>>,
    RuntimeServiceId: AsServiceId<Cryptarchia> + std::fmt::Debug + std::fmt::Display + Sync,
{
    async fn handle_wallet_message(
        msg: WalletMsg,
        wallet: &mut Wallet,
        storage_adapter: &StorageAdapter<Storage, Tx, RuntimeServiceId>,
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
                Self::get_leader_aged_notes(tip, tx, wallet, storage_adapter, cryptarchia_api)
                    .await;
            }
        }
    }

    async fn get_leader_aged_notes(
        tip: HeaderId,
        tx: oneshot::Sender<Result<Vec<Utxo>, WalletServiceError>>,
        wallet: &mut Wallet,
        storage_adapter: &StorageAdapter<Storage, Tx, RuntimeServiceId>,
        cryptarchia_api: &CryptarchiaServiceApi<Cryptarchia, RuntimeServiceId>,
    ) {
        // Get the ledger state at the specified tip
        let Ok(Some(ledger_state)) = cryptarchia_api.get_ledger_state(tip).await else {
            Self::send_err(tx, WalletServiceError::LedgerStateNotFound(tip));
            return;
        };

        let wallet_state = match wallet.wallet_state_at(tip) {
            Ok(wallet_state) => wallet_state,
            Err(WalletError::UnknownBlock) => {
                // There may be a race condition here where the caller knows a more recent
                // tip than the wallet. In that case, we will have received a
                // LedgerState for the tip from Cryptarchia, but we would be missing the
                // WalletState for that tip.
                //
                // To resolve this, we do a JIT backfill to try to sync the wallet with
                // cryptarchia. If that still can't find the corresponding wallet state
                // after the backfill, we return an error to the caller
                if let Err(e) =
                    Self::backfill_missing_blocks(tip, wallet, storage_adapter, cryptarchia_api)
                        .await
                {
                    error!(
                        err = ?e,
                        "Failed to backfill wallet while fetching aged notes"
                    );

                    Self::send_err(tx, e);
                    return;
                }

                let Ok(wallet_state) = wallet.wallet_state_at(tip) else {
                    Self::send_err(tx, WalletServiceError::WalletStateNotFound(tip));
                    return;
                };

                wallet_state
            }
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

    async fn backfill_missing_blocks(
        missing_block: HeaderId,
        wallet: &mut Wallet,
        storage_adapter: &StorageAdapter<Storage, Tx, RuntimeServiceId>,
        cryptarchia_api: &CryptarchiaServiceApi<Cryptarchia, RuntimeServiceId>,
    ) -> Result<(), WalletServiceError> {
        let headers = cryptarchia_api.get_headers_to_lib(missing_block).await?;

        debug!(
            backfill_size = headers.len(),
            "Received headers for backfill"
        );

        for header_id in headers.into_iter().rev() {
            let Some(block) = storage_adapter.get_block(&header_id).await else {
                error!(block_id = ?header_id, "Block not found in storage during wallet sync");
                return Err(WalletServiceError::BlockNotFoundInStorage(header_id));
            };

            if let Err(e) = wallet.apply_block(&block.into()) {
                error!(
                    block_id = ?header_id,
                    err = %e,
                    "Failed to apply backfill block to wallet"
                );
                return Err(WalletServiceError::BackfillFailedToApplyBlock(header_id));
            }
        }

        Ok(())
    }

    fn send_err<T: std::fmt::Debug>(
        tx: oneshot::Sender<Result<T, WalletServiceError>>,
        err: WalletServiceError,
    ) {
        if let Err(msg) = tx.send(Err(err)) {
            error!(msg = ?msg, "Wallet failed to send error response");
        }
    }
}
