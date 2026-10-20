//! Transfer proof construction and final Mantle transaction signing.

// TODO(RFC-480): UTXO test wallet.
// use std::collections::HashMap;
use std::collections::{BTreeMap, HashMap};

// TODO(RFC-480): UTXO test wallet.
// use lb_core::mantle::{
//     NoteId, Op, OpProof, OpRef, SignedOps, TxHash,
//     gas::{MainnetGasProfile, TxGasCalculator as _},
//     traits::Hashable as _,
//     transactions::{MantleTxBuilder, OpProofs, Ops, tx_list::ops::OpsContext},
// };
// use lb_key_management_system_service::keys::{Ed25519Key, ZkKey};
use lb_core::mantle::{
    Op, OpProof, OpRef, SignedOps, TxHash,
    gas::{MainnetGasProfile, TxGasCalculator as _},
    notes::NoteNf,
    ops::{ChannelInscribeProof, ZkTransferProof, transfer::prove_transfer},
    traits::Hashable as _,
    transactions::{
        MantleTxBuilder, OpProofs, Ops, builder::ZkTransferOpening, tx_list::ops::OpsContext,
    },
};
use lb_key_management_system_service::keys::{Ed25519Key, UnsecuredZkKey, ZkKey, ZkPublicKey};
use lb_wallet::{
    KnownNotes, OwnedNote, WalletState,
    transaction::{WitnessContext, op_witnesses},
};

use super::{error::WalletTransactionError, signed::SignedWalletTransaction};
use crate::common::wallet::WalletReservedInputs;

// TODO(RFC-480): UTXO test wallet, a note is spent by its nullifier and proven
// with its path.
// pub(super) type WalletTransferSigners = HashMap<NoteId, ZkKey>;
pub(super) type WalletTransferSigners = HashMap<NoteNf, (OwnedNote, ZkKey)>;

pub(super) fn sign_prepared_wallet_transaction(
    funded_builder: MantleTxBuilder,
    context: &OpsContext,
    tx_hash: TxHash,
    transfer_proofs: OpProofs,
    reserved_inputs: WalletReservedInputs,
    leading_op_proofs: OpProofs,
) -> Result<SignedWalletTransaction, WalletTransactionError> {
    let mantle_tx = funded_builder.build()?;
    let op_proofs = {
        let mut leading_op_proofs_vec = leading_op_proofs.into_inner().into_inner();
        leading_op_proofs_vec.extend(transfer_proofs);
        OpProofs::try_from(leading_op_proofs_vec)?
    };

    let signed_tx = SignedOps::from_parts(mantle_tx, op_proofs)?.preverify()?;
    let mandatory_fee_at_preparation = signed_tx
        .op_refs()
        .total_gas_cost::<MainnetGasProfile>(&context.gas_context)?
        .into_inner();
    // TODO(RFC-480): UTXO test wallet, the outputs are commitments and the
    // excess of a Transfer is what it pays.
    // let output_total = signed_tx
    //     .op_refs()
    //     .into_iter()
    //     .filter_map(|op| match op {
    //         OpRef::Transfer(transfer) => Some(transfer),
    //         _ => None,
    //     })
    //     .flat_map(|transfer| transfer.outputs.iter())
    //     .try_fold(0u64, |total, note| {
    //         total
    //             .checked_add(note.value)
    //             .ok_or(WalletTransactionError::OutputTotalOverflow)
    //     })?;
    // let paid_fee = reserved_inputs
    //     .total_value()
    //     .checked_sub(output_total)
    //     .ok_or(WalletTransactionError::FeeAccounting)?;
    let paid_fee = signed_tx
        .op_refs()
        .into_iter()
        .filter_map(|op| match op {
            OpRef::Transfer(transfer) => Some(transfer.excess_value),
            _ => None,
        })
        .try_fold(0u64, |total, excess| {
            total
                .checked_add(excess)
                .ok_or(WalletTransactionError::FeeAccounting)
        })?;

    Ok(SignedWalletTransaction::new(
        signed_tx,
        tx_hash,
        reserved_inputs,
        paid_fee,
        mandatory_fee_at_preparation,
    ))
}

// TODO(RFC-480): UTXO test wallet. A ZkTransfer replaces the ZkSignature.
/*
/// Build one `ZkSig` proof per transfer op in a funded transaction, signing
/// every input with the same wallet key. Suitable for transactions whose
/// funding inputs all come from a single wallet account.
pub fn transfer_proofs_for_funded_wallet_tx(
    tx: &Ops,
    signing_key: &ZkKey,
) -> Result<OpProofs, WalletTransactionError> {
    let tx_hash = tx.hash();
    let proofs = tx
        .into_iter()
        .filter_map(|op| match op {
            Op::Transfer(transfer_op) => Some(transfer_op),
            _ => None,
        })
        .map(|transfer_op| -> Result<OpProof, WalletTransactionError> {
            let signing_keys = transfer_op
                .inputs
                .iter()
                .map(|_| signing_key.clone())
                .collect::<Vec<_>>();
            Ok(OpProof::ZkSig(ZkKey::multi_sign(
                &signing_keys,
                &tx_hash.to_fr(),
            )?))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OpProofs::try_from(proofs).expect("transaction proofs are bounded"))
}
*/

/// Build the `ZkTransfer` proof of every Operation of a funded transaction
/// that spends, keyed by its index, from the witnesses of `state`, each input
/// proven with the key of its owner.
#[expect(
    clippy::implicit_hasher,
    reason = "The keys are the ones a scanner accounting holds, with the default hasher."
)]
pub fn transfer_proofs_for_funded_wallet_tx(
    state: &WalletState,
    known: &KnownNotes,
    tx: &Ops,
    openings: &BTreeMap<usize, ZkTransferOpening>,
    witness_context: &WitnessContext,
    signing_keys: &HashMap<ZkPublicKey, UnsecuredZkKey>,
) -> Result<BTreeMap<usize, ZkTransferProof>, WalletTransactionError> {
    let tx_hash = tx.hash();
    op_witnesses(state, known, tx, openings, witness_context, tx_hash.to_fr())?
        .into_iter()
        .map(|witness| -> Result<_, WalletTransactionError> {
            let keys = witness
                .signers
                .iter()
                .map(|public_key| {
                    signing_keys.get(public_key).cloned().ok_or(
                        WalletTransactionError::MissingSigningKey {
                            public_key: *public_key,
                        },
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((
                witness.op_index,
                ZkTransferProof::new(UnsecuredZkKey::prove_transfer(&keys, witness.witness)?),
            ))
        })
        .collect()
}

pub(super) fn build_transfer_proofs(
    ops: &[Op],
    // TODO(RFC-480): UTXO test wallet, a ZkTransfer proves the openings of its
    // notes.
    openings: &BTreeMap<usize, ZkTransferOpening>,
    tx_hash: &TxHash,
    transfer_signers: &WalletTransferSigners,
) -> Result<OpProofs, WalletTransactionError> {
    let proofs = ops
        .iter()
        // TODO(RFC-480): UTXO test wallet.
        // .filter_map(|op| match op {
        //     Op::Transfer(transfer_op) => Some(transfer_op),
        //     _ => None,
        // })
        .enumerate()
        .filter_map(|(op_index, op)| match op {
            Op::Transfer(transfer_op) => Some((op_index, transfer_op)),
            _ => None,
        })
        // .map(|transfer_op| -> Result<OpProof, WalletTransactionError> {
        //     let signing_keys = transfer_op
        //         .inputs
        //         .iter()
        //         .map(|note_id| {
        //             transfer_signers
        //                 .get(note_id)
        //                 .cloned()
        //                 .ok_or(WalletTransactionError::MissingSigningKey { note_id: *note_id })
        //         })
        //         .collect::<Result<Vec<_>, _>>()?;
        //
        //     Ok(OpProof::ZkSig(ZkKey::multi_sign(
        //         &signing_keys,
        //         &tx_hash.to_fr(),
        //     )?))
        // })
        .map(|(op_index, transfer_op)| -> Result<OpProof, WalletTransactionError> {
            let opening = openings
                .get(&op_index)
                .ok_or(lb_wallet::WalletError::MissingOpening { op_index })?;
            // Each input is proven with the path of its funding note, to the
            // root the funding Transfer names; no earlier Operation of a test
            // wallet transaction appends to the ledger set.
            let inputs = transfer_op
                .inputs
                .iter()
                .map(|nullifier| {
                    transfer_signers
                        .get(nullifier)
                        .map(|(owned, signing_key)| {
                            (owned.note, *signing_key.as_fr(), owned.path.clone())
                        })
                        .ok_or(WalletTransactionError::MissingFundingInput {
                            nullifier: *nullifier,
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;

            Ok(OpProof::ZkTransfer(ZkTransferProof::new(prove_transfer(
                &inputs,
                &opening.outputs,
                transfer_op.cm_merkle_root,
                tx_hash.to_fr(),
            ))))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OpProofs::try_from(proofs).expect("transaction proofs are bounded"))
}

pub(super) fn build_leading_inscription_proofs(
    ops: &[Op],
    tx_hash: &TxHash,
    signing_keys: &[Ed25519Key],
) -> Result<OpProofs, WalletTransactionError> {
    let proofs = ops
        .iter()
        .take(signing_keys.len())
        .zip(signing_keys)
        .map(|(op, signing_key)| {
            let Op::ChannelInscribe(inscription) = op else {
                return Err(WalletTransactionError::InvalidLeadingInscriptionSigner);
            };
            if inscription.signer != signing_key.public_key().into_unverified() {
                return Err(WalletTransactionError::InvalidLeadingInscriptionSigner);
            }

            // TODO(RFC-480): an inscription carries the proofs of its steps.
            // Ok(OpProof::Ed25519Sig(
            //     signing_key.sign_payload(tx_hash.as_signing_bytes()),
            // ))
            Ok(OpProof::ChannelInscribe(
                ChannelInscribeProof::without_steps(
                    signing_key.sign_payload(tx_hash.as_signing_bytes()),
                ),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if proofs.len() != signing_keys.len() {
        return Err(WalletTransactionError::InvalidLeadingInscriptionSigner);
    }

    Ok(OpProofs::try_from(proofs).expect("transaction proofs are bounded"))
}

#[cfg(test)]
mod tests {
    use lb_core::mantle::ops::channel::{
        ChannelId, MsgId,
        inscribe::{ChannelSteps, Inscription, InscriptionOp},
    };

    use super::*;

    #[test]
    fn leading_inscription_signature_uses_the_complete_transaction_hash() {
        let signing_key = Ed25519Key::from_bytes(&[0x42; 32]);
        let operation = InscriptionOp {
            channel_id: ChannelId::from([0x24; 32]),
            inscription: Inscription::try_from(1u64.to_le_bytes().to_vec())
                .expect("small fixed-width payload"),
            parent: MsgId::root(),
            signer: signing_key.public_key().into_unverified(),
            steps: ChannelSteps::default(),
        };
        let tx_hash = TxHash::default();
        let proofs = build_leading_inscription_proofs(
            &[Op::ChannelInscribe(operation)],
            &tx_hash,
            std::slice::from_ref(&signing_key),
        )
        .expect("inscription proof");
        // TODO(RFC-480): an inscription carries the proofs of its steps.
        // let Some(OpProof::Ed25519Sig(signature)) = proofs.into_inner().into_inner().pop() else {
        let Some(OpProof::ChannelInscribe(ChannelInscribeProof { signature, .. })) =
            proofs.into_inner().into_inner().pop()
        else {
            panic!("inscription proof should use Ed25519");
        };

        signing_key
            .public_key()
            .verify(tx_hash.as_signing_bytes(), &signature)
            .expect("signature should cover the transaction hash");
    }
}
