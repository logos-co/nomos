use nomos_core::{
    header::HeaderId,
    mantle::{Utxo, Value, tx_builder::MantleTxBuilder},
};
use overwatch::services::{
    AsServiceId, ServiceData,
    relay::{OutboundRelay, RelayError},
};
use tokio::sync::oneshot::{self, error::RecvError};
use zksign::PublicKey;

use crate::{WalletMsg, WalletServiceError, WalletServiceSettings};

#[derive(Debug, thiserror::Error)]
pub enum WalletApiError {
    #[error("Failed to relay message with wallet:{relay_error:?}, msg={msg:?}")]
    RelaySend {
        relay_error: RelayError,
        msg: WalletMsg,
    },
    #[error("Failed to recv message from wallet: {0}")]
    RelayRecv(#[from] RecvError),
    #[error(transparent)]
    Wallet(#[from] WalletServiceError),
}

impl From<(RelayError, WalletMsg)> for WalletApiError {
    fn from((relay_error, msg): (RelayError, WalletMsg)) -> Self {
        Self::RelaySend { relay_error, msg }
    }
}

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

impl<Wallet, RuntimeServiceId> WalletApi<Wallet, RuntimeServiceId>
where
    Wallet: WalletServiceData,
    RuntimeServiceId: AsServiceId<Wallet> + std::fmt::Debug + std::fmt::Display + Sync,
{
    #[must_use]
    pub const fn new(relay: OutboundRelay<Wallet::Message>) -> Self {
        Self {
            relay,
            _id: std::marker::PhantomData,
        }
    }

    pub async fn get_balance(
        &self,
        tip: Option<HeaderId>,
        pk: PublicKey,
    ) -> Result<Option<Value>, WalletApiError> {
        let (resp_tx, rx) = oneshot::channel();

        self.relay
            .send(WalletMsg::GetBalance { tip, pk, resp_tx })
            .await?;

        Ok(rx.await??)
    }

    pub async fn fund_tx(
        &self,
        tip: Option<HeaderId>,
        tx_builder: MantleTxBuilder,
        change_pk: PublicKey,
        funding_pks: Vec<PublicKey>,
    ) -> Result<MantleTxBuilder, WalletApiError> {
        let (resp_tx, rx) = oneshot::channel();

        self.relay
            .send(WalletMsg::FundTx {
                tip,
                tx_builder,
                change_pk,
                funding_pks,
                resp_tx,
            })
            .await?;

        Ok(rx.await??)
    }

    pub async fn sign_tx(
        &self,
        tip: Option<HeaderId>,
        tx_builder: MantleTxBuilder,
    ) -> Result<nomos_core::mantle::SignedMantleTx, WalletApiError> {
        let (resp_tx, rx) = oneshot::channel();

        self.relay
            .send(WalletMsg::SignTx {
                tip,
                tx_builder,
                resp_tx,
            })
            .await?;

        Ok(rx.await??)
    }

    pub async fn get_leader_aged_notes(
        &self,
        tip: Option<HeaderId>,
    ) -> Result<Vec<Utxo>, WalletApiError> {
        let (resp_tx, rx) = oneshot::channel();

        self.relay
            .send(WalletMsg::GetLeaderAgedNotes { tip, resp_tx })
            .await?;

        Ok(rx.await??)
    }
}
