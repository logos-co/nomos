pub mod error;
pub mod wallet;

use std::collections::HashSet;

use async_trait::async_trait;
use chain_service::{ConsensusMsg, CryptarchiaInfo};
use kzgrs_backend::dispersal::BlobInfo;
use nomos_core::{
    header::HeaderId,
    mantle::{keys::PublicKey, Utxo, Value},
};
use nomos_ledger::LedgerState;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use tokio::sync::oneshot;

use error::WalletError;
use wallet::Wallet;

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
}

#[derive(Clone, Debug)]
pub struct WalletServiceSettings {
    pub known_keys: HashSet<PublicKey>,
}

pub struct WalletService<CryptarchiaService, RuntimeServiceId>
where
    CryptarchiaService: ServiceData,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
}

impl<CryptarchiaService, RuntimeServiceId> ServiceData
    for WalletService<CryptarchiaService, RuntimeServiceId>
where
    CryptarchiaService: ServiceData,
{
    type Settings = WalletServiceSettings;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = WalletMsg;
}

#[async_trait]
impl<CryptarchiaService, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for WalletService<CryptarchiaService, RuntimeServiceId>
where
    CryptarchiaService: ServiceData + Send + 'static,
    CryptarchiaService::Message: From<ConsensusMsg<nomos_core::block::Block<nomos_core::mantle::SignedMantleTx, BlobInfo>>>
        + Send,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<CryptarchiaService>
        + Clone
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
        })
    }

    async fn run(mut self) -> Result<(), DynError> {
        let Self {
            service_resources_handle,
        } = self;

        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let status_updater = service_resources_handle.status_updater;
        let mut inbound_relay = service_resources_handle.inbound_relay;
        let overwatch_handle = service_resources_handle.overwatch_handle.clone();

        let chain_relay = overwatch_handle.relay::<CryptarchiaService>().await?;

        // Query chain service for current state
        let (info_tx, info_rx) = oneshot::channel();
        chain_relay
            .send(ConsensusMsg::Info { tx: info_tx }.into())
            .await
            .map_err(|e| format!("Failed to query chain info: {:?}", e.0))?;

        let chain_info: CryptarchiaInfo = info_rx
            .await
            .map_err(|e| format!("Failed to receive chain info: {:?}", e))?;

        eprintln!(
            "Wallet connecting to chain at tip: {:?}, lib: {:?}, slot: {:?}",
            chain_info.tip, chain_info.lib, chain_info.slot
        );

        // Subscribe to block updates
        let (block_subscription_tx, block_subscription_rx) = oneshot::channel();
        chain_relay
            .send(
                ConsensusMsg::BlockSubscribe {
                    sender: block_subscription_tx,
                }
                .into(),
            )
            .await
            .map_err(|e| format!("Failed to subscribe to blocks: {:?}", e.0))?;

        let mut block_receiver = block_subscription_rx
            .await
            .map_err(|e| format!("Failed to receive block subscription: {:?}", e))?;

        // Initialize wallet from LIB and LIB LedgerState
        let lib = chain_info.lib;

        let (lib_state_tx, lib_state_rx) = oneshot::channel();
        chain_relay
            .send(
                ConsensusMsg::GetLedgerState {
                    block_id: lib,
                    tx: lib_state_tx,
                }
                .into(),
            )
            .await
            .map_err(|e| format!("Failed to request LIB ledger state: {:?}", e.0))?;

        let lib_ledger = lib_state_rx
            .await
            .map_err(|e| format!("Failed to receive LIB ledger state: {:?}", e))?
            .unwrap_or_else(|| {
                eprintln!("Warning: LIB ledger state not found, using empty state");
                LedgerState::from_utxos([])
            });

        let mut wallet = Wallet::from_lib(settings.known_keys.clone(), lib, lib_ledger);

        status_updater.notify_ready();
        eprintln!("Wallet service is ready and subscribed to blocks");

        loop {
            tokio::select! {
                Some(msg) = inbound_relay.recv() => {
                    Self::handle_wallet_message(msg, &mut wallet).await;
                }
                Ok(block) = block_receiver.recv() => {
                    let wallet_block = wallet::WalletBlock::from(block);
                    match wallet.apply_block(&wallet_block) {
                        Ok(()) => {
                            eprintln!("Applied block {:?} to wallet", wallet_block.id);
                        }
                        Err(e) => {
                            eprintln!("Failed to apply block to wallet: {:?}", e);
                        }
                    }
                }
            }
        }
    }
}

impl<CryptarchiaService, RuntimeServiceId> WalletService<CryptarchiaService, RuntimeServiceId>
where
    CryptarchiaService: ServiceData,
{
    async fn handle_wallet_message(msg: WalletMsg, wallet: &mut Wallet) {
        match msg {
            WalletMsg::GetBalance { tip, pk, tx } => {
                let result = wallet.balance(tip, pk);
                if tx.send(result).is_err() {
                    eprintln!("Failed to send balance response");
                }
            }
            WalletMsg::GetUtxosForAmount {
                tip,
                amount,
                pks,
                tx,
            } => {
                let result = wallet.utxos_for_amount(tip, amount, pks);
                if tx.send(result).is_err() {
                    eprintln!("Failed to send UTXOs response");
                }
            }
        }
    }
}
