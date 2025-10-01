use nomos_core::{
    header::HeaderId,
    mantle::{Utxo, Value, keys::PublicKey},
};
use overwatch::{
    DynError,
    services::{AsServiceId, ServiceData, relay::OutboundRelay},
};
use tokio::sync::oneshot;

use crate::{WalletMsg, WalletServiceSettings};

pub trait WalletServiceData:
    ServiceData<Settings = WalletServiceSettings, Message = WalletMsg>
{
    type Cryptarchia;
    type Tx;
    type Storage;
}

impl<Cryptarchia, Tx, Storage, RuntimeServiceId> WalletServiceData
    for crate::WalletService<Cryptarchia, Tx, Storage, RuntimeServiceId>
{
    type Cryptarchia = Cryptarchia;
    type Tx = Tx;
    type Storage = Storage;
}

pub struct WalletApi<Wallet, RuntimeServiceId>
where
    Wallet: WalletServiceData,
{
    relay: OutboundRelay<Wallet::Message>,
    _id: std::marker::PhantomData<RuntimeServiceId>,
}

impl<Wallet, RuntimeServiceId> From<OutboundRelay<Wallet::Message>>
    for WalletApi<Wallet, RuntimeServiceId>
where
    Wallet: WalletServiceData,
{
    fn from(relay: OutboundRelay<Wallet::Message>) -> Self {
        Self {
            relay,
            _id: std::marker::PhantomData,
        }
    }
}

impl<Wallet, RuntimeServiceId> WalletApi<Wallet, RuntimeServiceId>
where
    Wallet: WalletServiceData,
    RuntimeServiceId: AsServiceId<Wallet> + std::fmt::Debug + std::fmt::Display + Sync,
{
    #[must_use]
    pub fn new(relay: OutboundRelay<Wallet::Message>) -> Self {
        Self::from(relay)
    }

    pub async fn get_balance(
        &self,
        tip: HeaderId,
        pk: PublicKey,
    ) -> Result<Option<Value>, DynError> {
        let (tx, rx) = oneshot::channel();

        self.relay
            .send(WalletMsg::GetBalance { tip, pk, tx })
            .await
            .map_err(|e| format!("Failed to send balance request: {e:?}"))?;

        Ok(rx.await??)
    }

    pub async fn get_utxos_for_amount(
        &self,
        tip: HeaderId,
        amount: Value,
        pks: Vec<PublicKey>,
    ) -> Result<Option<Vec<Utxo>>, DynError> {
        let (tx, rx) = oneshot::channel();

        self.relay
            .send(WalletMsg::GetUtxosForAmount {
                tip,
                amount,
                pks,
                tx,
            })
            .await
            .map_err(|e| format!("Failed to send get_utxos_for_amount request: {e:?}"))?;

        Ok(rx.await??)
    }

    pub async fn get_leader_aged_notes(&self, tip: HeaderId) -> Result<Vec<Utxo>, DynError> {
        let (tx, rx) = oneshot::channel();

        self.relay
            .send(WalletMsg::GetLeaderAgedNotes { tip, tx })
            .await
            .map_err(|e| format!("Failed to send get_leader_aged_notes request: {e:?}"))?;

        Ok(rx.await??)
    }
}
