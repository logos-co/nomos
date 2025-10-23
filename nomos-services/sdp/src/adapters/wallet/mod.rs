pub mod mock;

use nomos_core::{
    mantle::{SignedMantleTx, tx_builder::MantleTxBuilder},
    sdp::{ActiveMessage, DeclarationMessage, WithdrawMessage},
};

#[async_trait::async_trait]
pub trait SdpWalletAdapter {
    type Error;

    // TODO: Pass relay when wallet service is defined.
    fn new() -> Self;

    fn declare_tx(
        &self,
        tx_builder: MantleTxBuilder,
        declaration: Box<DeclarationMessage>,
    ) -> Result<SignedMantleTx, Self::Error>;

    fn withdraw_tx(
        &self,
        tx_builder: MantleTxBuilder,
        withdrawn_message: WithdrawMessage,
    ) -> Result<SignedMantleTx, Self::Error>;

    fn active_tx(
        &self,
        tx_builder: MantleTxBuilder,
        active_message: ActiveMessage,
    ) -> Result<SignedMantleTx, Self::Error>;
}
