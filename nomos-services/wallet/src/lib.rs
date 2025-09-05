pub mod error;
pub mod wallet;

use std::collections::HashSet;

use async_trait::async_trait;
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
    RuntimeServiceId: AsServiceId<Self> + Send + Sync + 'static,
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

        // TODO: Query cryptarchia for LIB and LIB LedgerState
        // For now, use genesis as placeholder
        let lib = HeaderId::from([0u8; 32]);
        let lib_ledger = LedgerState::from_utxos([]);
        let mut wallet = Wallet::from_lib(settings.known_keys.clone(), lib, lib_ledger);

        status_updater.notify_ready();
        eprintln!("Wallet service is ready");

        // TODO: Add consensus block subscription here
        // For now, just handle wallet query messages

        // Handle wallet messages
        while let Some(msg) = inbound_relay.recv().await {
            Self::handle_wallet_message(msg, &mut wallet).await;
        }

        Ok(())
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
