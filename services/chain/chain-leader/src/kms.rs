use std::fmt::{Debug, Display};

// TODO(RFC-480): the lottery runs on notes; the KMS fills the secret key of a
// wallet witness built from the note sets.
// use lb_core::{
//     mantle::Utxo,
//     proofs::leader_proof::{LeaderPrivate, LeaderPublic},
// };
// use lb_key_management_system_service::{
//     KMSService,
//     api::KmsServiceApi,
//     backend::preload::{KeyId, PreloadKMSBackend},
//     keys::{Ed25519Key, KeyOperators},
//     operators::zk::leader::CheckLotteryWinning,
// };
// use lb_ledger::{EpochState, UtxoTree};
use lb_core::{
    mantle::Note,
    proofs::leader_proof::{LeaderPrivate, LeaderPublic},
};
use lb_groth16::Fr;
use lb_key_management_system_service::{
    KMSService,
    api::KmsServiceApi,
    backend::preload::{KeyId, PreloadKMSBackend},
    keys::KeyOperators,
    operators::zk::leader::{BuildPrivateInputsWithLeaderKey, CheckLotteryWinning},
};
use lb_pol::PolWalletInputsData;
use overwatch::{DynError, services::AsServiceId};
use tokio::sync::oneshot;

// TODO(RFC-480): the KMS fills the secret key of a wallet witness built from the note sets.
// use crate::leadership::{
//     PrivateInputsError, operator_for_private_inputs_arguments_for_winning_utxo_and_slot,
// };
use crate::leadership::PrivateInputsError;

pub type PreloadKmsService<RuntimeServiceId> = KMSService<PreloadKMSBackend, RuntimeServiceId>;

#[async_trait::async_trait]
pub trait KmsAdapter<RuntimeServiceId> {
    type KeyId;

    // TODO(RFC-480): the lottery runs on notes.
    // async fn check_winning_with_key(
    //     &self,
    //     key_id: Self::KeyId,
    //     utxo: &Utxo,
    //     public_inputs: &LeaderPublic,
    // ) -> Result<bool, DynError>;
    //
    // async fn build_private_inputs_for_winning_utxo_and_slot(
    //     &self,
    //     key_id: Self::KeyId,
    //     utxo: &Utxo,
    //     epoch_state: &EpochState,
    //     public_inputs: LeaderPublic,
    //     latest_tree: &UtxoTree,
    // ) -> Result<(LeaderPrivate, Ed25519Key), PrivateInputsError>;
    async fn check_winning_with_key(
        &self,
        key_id: Self::KeyId,
        note: &Note,
        public_inputs: &LeaderPublic,
    ) -> Result<bool, DynError>;

    async fn build_private_inputs(
        &self,
        key_id: Self::KeyId,
        public_inputs: LeaderPublic,
        wallet: PolWalletInputsData,
        msg: Fr,
    ) -> Result<LeaderPrivate, PrivateInputsError>;
}

#[async_trait::async_trait]
impl<RuntimeServiceId> KmsAdapter<RuntimeServiceId>
    for KmsServiceApi<PreloadKmsService<RuntimeServiceId>, RuntimeServiceId>
where
    RuntimeServiceId:
        AsServiceId<PreloadKmsService<RuntimeServiceId>> + Debug + Display + Send + Sync + 'static,
{
    type KeyId = KeyId;

    async fn check_winning_with_key(
        &self,
        key_id: Self::KeyId,
        // TODO(RFC-480): the lottery runs on notes.
        // utxo: &Utxo,
        note: &Note,
        public_inputs: &LeaderPublic,
    ) -> Result<bool, DynError> {
        let (output_tx, output_rx) = oneshot::channel();
        // clone to send
        // TODO(RFC-480): the lottery runs on notes.
        // let utxo = *utxo;
        let note = *note;
        let public_inputs = *public_inputs;
        Box::pin(self.execute(
            key_id,
            KeyOperators::Zk(Box::new(CheckLotteryWinning::new(
                output_tx,
                // TODO(RFC-480): the lottery runs on notes.
                // utxo,
                note,
                public_inputs,
            ))),
        ))
        .await?;

        Ok(output_rx
            .await
            .map_err(|_| "KMS API did not respond to the lottery winning check")?)
    }

    // TODO(RFC-480): the KMS fills the secret key of a wallet witness built from the note sets.
    // async fn build_private_inputs_for_winning_utxo_and_slot(
    //     &self,
    //     key_id: Self::KeyId,
    //     utxo: &Utxo,
    //     epoch_state: &EpochState,
    //     public_inputs: LeaderPublic,
    //     latest_tree: &UtxoTree,
    // ) -> Result<(LeaderPrivate, Ed25519Key), PrivateInputsError> {
    //     // clone to send
    //     let utxo = *utxo;
    //     let epoch_state = epoch_state.clone();
    //     let latest_tree = latest_tree.clone();
    //     let (operator, output_rx, key) =
    //         operator_for_private_inputs_arguments_for_winning_utxo_and_slot(
    //             &utxo,
    //             &epoch_state,
    //             public_inputs,
    //             &latest_tree,
    //         )?;
    //     self.execute(key_id, KeyOperators::Zk(Box::new(operator)))
    //         .await
    //         .map_err(|error| PrivateInputsError::KmsApi(Box::new(error)))?;
    //
    //     Ok((
    //         output_rx
    //             .await
    //             .map_err(|_| PrivateInputsError::KmsResponse)?,
    //         key,
    //     ))
    // }
    async fn build_private_inputs(
        &self,
        key_id: Self::KeyId,
        public_inputs: LeaderPublic,
        wallet: PolWalletInputsData,
        msg: Fr,
    ) -> Result<LeaderPrivate, PrivateInputsError> {
        let (output_tx, output_rx) = oneshot::channel();
        let operator = BuildPrivateInputsWithLeaderKey::new(output_tx, public_inputs, wallet, msg);
        Box::pin(self.execute(key_id, KeyOperators::Zk(Box::new(operator))))
            .await
            .map_err(|error| PrivateInputsError::KmsApi(Box::new(error)))?;

        output_rx
            .await
            .map_err(|_| PrivateInputsError::KmsResponse)
    }
}
