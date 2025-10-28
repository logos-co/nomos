use nomos_core::{
    header::HeaderId,
    mantle::{Utxo, Value, keys::PublicKey, tx_builder::MantleTxBuilder},
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
    type Kms;
    type Cryptarchia;
    type Tx;
    type Storage;
}

impl<Kms, Cryptarchia, Tx, Storage, RuntimeServiceId> WalletServiceData
    for crate::WalletService<Kms, Cryptarchia, Tx, Storage, RuntimeServiceId>
{
    type Kms = Kms;
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
        let (resp_tx, rx) = oneshot::channel();

        self.relay
            .send(WalletMsg::GetBalance { tip, pk, resp_tx })
            .await
            .map_err(|e| format!("Failed to send balance request: {e:?}"))?;

        Ok(rx.await??)
    }

    pub async fn fund_and_sign_tx(
        &self,
        tip: HeaderId,
        tx_builder: MantleTxBuilder,
        change_pk: PublicKey,
        funding_pks: Vec<PublicKey>,
    ) -> Result<nomos_core::mantle::SignedMantleTx, DynError> {
        let (resp_tx, rx) = oneshot::channel();

        self.relay
            .send(WalletMsg::FundAndSignTx {
                tip,
                tx_builder,
                change_pk,
                funding_pks,
                resp_tx,
            })
            .await
            .map_err(|e| format!("Failed to send fund_and_sign_tx request: {e:?}"))?;

        Ok(rx.await??)
    }

    pub async fn get_leader_aged_notes(&self, tip: HeaderId) -> Result<Vec<Utxo>, DynError> {
        let (resp_tx, rx) = oneshot::channel();

        self.relay
            .send(WalletMsg::GetLeaderAgedNotes { tip, resp_tx })
            .await
            .map_err(|e| format!("Failed to send get_leader_aged_notes request: {e:?}"))?;

        Ok(rx.await??)
    }
}
