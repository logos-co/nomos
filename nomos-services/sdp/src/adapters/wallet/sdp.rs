use std::fmt::{Debug, Display};

use nomos_core::{
    mantle::{Op, SignedMantleTx, gas::MainnetGasConstants, tx_builder::MantleTxBuilder},
    sdp::{ActiveMessage, DeclarationMessage, WithdrawMessage},
};
use nomos_wallet::api::{WalletApi, WalletServiceData};
use overwatch::services::AsServiceId;

use super::{SdpWalletAdapter, SdpWalletConfig, SdpWalletError};

impl<Wallet, RuntimeServiceId> SdpWalletAdapter for WalletApi<Wallet, RuntimeServiceId>
where
    Wallet: WalletServiceData,
    RuntimeServiceId: AsServiceId<Wallet> + Debug + Display + Sync,
{
    async fn declare_tx(
        &self,
        mut tx_builder: MantleTxBuilder,
        declaration: DeclarationMessage,
        config: &SdpWalletConfig,
    ) -> Result<SignedMantleTx, SdpWalletError> {
        tx_builder = tx_builder.push_op(Op::SDPDeclare(declaration));

        let funded = self
            .fund_tx(None, tx_builder, config.funding_pk, vec![config.funding_pk])
            .await?;

        let tx_fee = funded.gas_cost::<MainnetGasConstants>();
        if tx_fee > config.max_tx_fee {
            return Err(SdpWalletError::TxFeeExceedsMaxFee {
                tx_fee,
                max_fee: config.max_tx_fee,
            });
        }

        let signed_tx = self.sign_tx(None, funded).await?;

        Ok(signed_tx)
    }

    async fn withdraw_tx(
        &self,
        mut tx_builder: MantleTxBuilder,
        withdraw: WithdrawMessage,
        config: &SdpWalletConfig,
    ) -> Result<SignedMantleTx, SdpWalletError> {
        tx_builder = tx_builder.push_op(Op::SDPWithdraw(withdraw));

        let funded = self
            .fund_tx(None, tx_builder, config.funding_pk, vec![config.funding_pk])
            .await?;

        let tx_fee = funded.gas_cost::<MainnetGasConstants>();
        if tx_fee > config.max_tx_fee {
            return Err(SdpWalletError::TxFeeExceedsMaxFee {
                tx_fee,
                max_fee: config.max_tx_fee,
            });
        }

        let signed_tx = self.sign_tx(None, funded).await?;

        Ok(signed_tx)
    }

    async fn active_tx(
        &self,
        mut tx_builder: MantleTxBuilder,
        active: ActiveMessage,
        config: &SdpWalletConfig,
    ) -> Result<SignedMantleTx, SdpWalletError> {
        tx_builder = tx_builder.push_op(Op::SDPActive(active));

        let funded = self
            .fund_tx(None, tx_builder, config.funding_pk, vec![config.funding_pk])
            .await?;

        let tx_fee = funded.gas_cost::<MainnetGasConstants>();
        if tx_fee > config.max_tx_fee {
            return Err(SdpWalletError::TxFeeExceedsMaxFee {
                tx_fee,
                max_fee: config.max_tx_fee,
            });
        }

        let signed_tx = self.sign_tx(None, funded).await?;

        Ok(signed_tx)
    }
}
