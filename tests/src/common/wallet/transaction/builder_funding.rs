//! Applies wallet funding decisions to Mantle transaction builders.

// TODO(RFC-480): UTXO test wallet.
// use std::{cmp::Ordering, collections::HashSet};
use std::cmp::Ordering;

// TODO(RFC-480): UTXO test wallet.
// use lb_core::mantle::{
//     Note, Op, Utxo,
//     gas::MainnetGasProfile,
//     ledger::{Inputs, Outputs},
//     ops::transfer::TransferOp,
//     transactions::{MantleTxBuilder, tx_list::ops::OpsContext},
// };
// use lb_key_management_system_service::keys::{Ed25519Key, MAX_ZK_SIGNING_KEYS, ZkPublicKey};
// use lb_mmr::MerkleMountainRange;
// use lb_wallet::{WalletError, WalletState};
// use rpds::{HashTrieMapSync, HashTrieSetSync};
use lb_core::{
    crypto::ZkHasher,
    mantle::{
        Note,
        gas::MainnetGasProfile,
        transactions::{MantleTxBuilder, builder::FundingNote, tx_list::ops::OpsContext},
    },
};
use lb_groth16::Fr;
use lb_key_management_system_service::keys::{Ed25519Key, ZkPublicKey};
use lb_wallet::{OwnedNote, WalletError};
use lb_zktransfer::MAX_INPUTS;

use super::{error::WalletTransactionError, intent::WalletTransactionIntent};
use crate::common::wallet::{
    WalletFundingOutcome, WalletFundingPlan, WalletFundingResources,
    // TODO(RFC-480): UTXO test wallet.
    // WalletFundingSource,
    WalletFundingUtxos, WalletInputSelectionStrategy, WalletSelectedInputs,
};

// The nonce of a note the test wallet creates, drawn at random so that no two
// notes of a key share a commitment.
#[must_use]
pub fn random_nonce() -> Fr {
    Fr::from(rand::random::<u128>())
}

// TODO(RFC-480): UTXO test wallet. A funding source carries notes with paths
// but no commitment MMR; `funded_signed_tx` funds with the `WalletState` it
// follows the chain with.
/*
pub fn fund_builder_from_wallet_source(
    source: &WalletFundingSource,
    tx_builder: &MantleTxBuilder,
    context: &OpsContext,
) -> Result<MantleTxBuilder, WalletError> {
    wallet_state_from_utxos(source.available_utxos().to_vec()).fund_tx::<MainnetGasProfile>(
        tx_builder,
        source.public_key(),
        [source.public_key()],
        context,
        &HashSet::new(),
        0,
    )
}

pub fn wallet_state_from_utxos(utxos: Vec<Utxo>) -> WalletState {
    let mut utxo_map = HashTrieMapSync::new_sync();
    let mut pk_index = HashTrieMapSync::new_sync();

    for utxo in utxos {
        let note_id = utxo.id();
        let pk = utxo.note.pk;
        utxo_map = utxo_map.insert(note_id, utxo);

        let note_set = pk_index
            .get(&pk)
            .cloned()
            .unwrap_or_else(HashTrieSetSync::new_sync)
            .insert(note_id);
        pk_index = pk_index.insert(pk, note_set);
    }

    WalletState {
        utxos: utxo_map,
        pk_index,
        service_notes: HashTrieSetSync::new_sync(),
        channel_notes: HashTrieSetSync::new_sync(),
        epoch: 0.into(),
        vouchers: MerkleMountainRange::new(),
        voucher_paths: HashTrieMapSync::new_sync(),
        voucher_paths_snapshot: HashTrieMapSync::new_sync(),
    }
}
*/

pub(super) fn fund_wallet_transaction(
    intent: WalletTransactionIntent,
    resources: WalletFundingResources,
    priority_fee_percent: u64,
) -> Result<(MantleTxBuilder, OpsContext, Vec<Ed25519Key>), WalletError> {
    let (sender, fee_sponsor) = resources.into_parts();
    let (tx_builder, context, sender_output_total, leading_inscription_signers) =
        intent.into_parts();

    let funded_builder = match fee_sponsor {
        None => {
            let (sender, input_selection_strategy) = sender.into_funding_parts();
            fund_unsponsored_wallet_transaction(
                &tx_builder,
                sender,
                input_selection_strategy,
                &context,
                priority_fee_percent,
            )?
        }
        Some(fee_sponsor) => {
            let (sender, input_selection_strategy) = sender.into_funding_parts();
            fund_sponsored_wallet_transaction(
                tx_builder,
                sender_output_total,
                fee_sponsor.into_funding_utxos(),
                sender,
                input_selection_strategy,
                &context,
                priority_fee_percent,
            )?
        }
    };

    Ok((funded_builder, context, leading_inscription_signers))
}

/// Estimate the exact gas reserve for a bounded continuous-workload input
/// candidate, both with and without a change output. This is diagnostic-only:
/// funding still goes through the normal transaction builder.
pub fn estimate_workload_fee_requirements(
    intent: &WalletTransactionIntent,
    candidate_inputs: &[OwnedNote],
    priority_fee_percent: u64,
) -> Result<(u64, u64), WalletTransactionError> {
    let (tx_builder, context, _, _) = intent.clone().into_parts();
    let builder_with_inputs = extend_wallet_funding_inputs(&tx_builder, candidate_inputs)?;
    let fee_without_change = required_fee_with_priority(
        builder_with_inputs
            .minimum_gas_cost::<MainnetGasProfile>(&context)?
            .into_inner(),
        priority_fee_percent,
    )?;
    // TODO(RFC-480): UTXO test wallet.
    // let builder_with_change =
    //     builder_with_inputs.add_ledger_output(Note::new(0, ZkPublicKey::zero()))?;
    let builder_with_change = builder_with_inputs.with_dummy_change_note()?;
    let fee_with_change = required_fee_with_priority(
        builder_with_change
            .minimum_gas_cost::<MainnetGasProfile>(&context)?
            .into_inner(),
        priority_fee_percent,
    )?;

    Ok((fee_without_change, fee_with_change))
}

fn required_fee_with_priority(
    mandatory_fee: u64,
    priority_fee_percent: u64,
) -> Result<u64, WalletTransactionError> {
    let mandatory_fee = u128::from(mandatory_fee);
    let priority_fee = mandatory_fee
        .checked_mul(u128::from(priority_fee_percent))
        .and_then(|fee| fee.checked_add(99))
        .ok_or(lb_core::mantle::gas::GasOverflow)?
        / 100;
    u64::try_from(mandatory_fee + priority_fee)
        .map_err(|_| lb_core::mantle::gas::GasOverflow.into())
}

fn fund_unsponsored_wallet_transaction(
    tx_builder: &MantleTxBuilder,
    sender: WalletFundingUtxos,
    input_selection_strategy: WalletInputSelectionStrategy,
    context: &OpsContext,
    priority_fee_percent: u64,
) -> Result<MantleTxBuilder, WalletError> {
    let sender_change_pk = sender.change_pk();
    let available_utxos = sender.into_available_utxos();
    let funding_plan = match input_selection_strategy {
        WalletInputSelectionStrategy::LargestFirst => {
            WalletFundingPlan::largest_first(available_utxos, Vec::new())
        }
        WalletInputSelectionStrategy::SmallestFirst => {
            WalletFundingPlan::smallest_first(available_utxos, Vec::new())
        }
        WalletInputSelectionStrategy::AllProvided => {
            WalletFundingPlan::all_provided(available_utxos)
        }
    };

    fund_builder_from_plan(
        tx_builder,
        sender_change_pk,
        &funding_plan,
        context,
        priority_fee_percent,
    )
}

fn fund_sponsored_wallet_transaction(
    tx_builder: MantleTxBuilder,
    output_total: u64,
    fee_sponsor: WalletFundingUtxos,
    sender: WalletFundingUtxos,
    input_selection_strategy: WalletInputSelectionStrategy,
    context: &OpsContext,
    priority_fee_percent: u64,
) -> Result<MantleTxBuilder, WalletError> {
    let sender_change_pk = sender.change_pk();
    let fee_sponsor_change_pk = fee_sponsor.change_pk();
    let sender_inputs = match input_selection_strategy {
        WalletInputSelectionStrategy::AllProvided => WalletSelectedInputs::all_provided_covering(
            sender.into_available_utxos(),
            output_total,
        )?,
        WalletInputSelectionStrategy::LargestFirst
        | WalletInputSelectionStrategy::SmallestFirst => {
            WalletSelectedInputs::largest_first_covering(
                sender.into_available_utxos(),
                output_total,
            )?
        }
    };
    let builder_with_sender_outputs = add_sender_change_output(
        tx_builder,
        sender_change_pk,
        output_total,
        sender_inputs.total(),
    )?;

    fund_builder_from_plan(
        &builder_with_sender_outputs,
        fee_sponsor_change_pk,
        &WalletFundingPlan::smallest_first(
            fee_sponsor.into_available_utxos(),
            sender_inputs.into_utxos(),
        ),
        context,
        priority_fee_percent,
    )
}

fn add_sender_change_output(
    tx_builder: MantleTxBuilder,
    change_pk: ZkPublicKey,
    output_total: u64,
    input_total: u64,
) -> Result<MantleTxBuilder, WalletError> {
    let change = input_total - output_total;
    if change > 0 {
        // TODO(RFC-480): UTXO test wallet.
        // tx_builder
        //     .add_ledger_output(Note::new(change, change_pk))
        tx_builder
            .add_ledger_output(Note::new(change, random_nonce(), change_pk))
            .map_err(Into::into)
    } else {
        Ok(tx_builder)
    }
}

fn fund_builder_from_plan(
    tx_builder: &MantleTxBuilder,
    change_pk: ZkPublicKey,
    plan: &WalletFundingPlan,
    context: &OpsContext,
    priority_fee_percent: u64,
) -> Result<MantleTxBuilder, WalletError> {
    plan.fund_with(|selected_inputs| {
        evaluate_funding_inputs(
            tx_builder,
            selected_inputs,
            change_pk,
            context,
            priority_fee_percent,
        )
    })
}

/// Add wallet funding inputs using the same transfer layout used during
/// transaction preparation.
///
/// The funding Transfer spends at most four notes, proven against the root
/// their paths lead to: the notes of one wallet observation share it.
pub fn extend_wallet_funding_inputs(
    tx_builder: &MantleTxBuilder,
    funding_utxos: &[OwnedNote],
) -> Result<MantleTxBuilder, WalletError> {
    // TODO(RFC-480): UTXO test wallet, a ZkTransfer spends at most four notes.
    // if funding_utxos.len() <= MAX_ZK_SIGNING_KEYS {
    //     return tx_builder
    //         .clone()
    //         .extend_ledger_inputs(funding_utxos.iter().copied())
    //         .map_err(Into::into);
    // }
    //
    // with_transfer_input_chunks(tx_builder, funding_utxos)
    let Some(first) = funding_utxos.first() else {
        return Ok(tx_builder.clone());
    };
    let funding_root = first.path.root::<ZkHasher>(first.note.commitment().0);
    tx_builder
        .clone()
        .extend_funding_inputs(
            funding_root,
            funding_utxos.iter().map(|utxo| FundingNote {
                note: utxo.note,
                nullifier: utxo.nullifier,
            }),
        )
        .map_err(Into::into)
}

fn evaluate_funding_inputs(
    tx_builder: &MantleTxBuilder,
    selected_inputs: &[OwnedNote],
    change_pk: ZkPublicKey,
    context: &OpsContext,
    priority_fee_percent: u64,
) -> Result<WalletFundingOutcome<MantleTxBuilder>, WalletError> {
    // TODO(RFC-480): UTXO test wallet.
    // if selected_inputs.is_empty() && tx_builder.ledger_inputs().is_empty() {
    if selected_inputs.is_empty() && tx_builder.funding_inputs().is_empty() {
        return Ok(WalletFundingOutcome::NeedsMoreInputs);
    }

    // TODO(RFC-480): UTXO test wallet, a ZkTransfer spends at most four notes.
    // if selected_inputs.len() <= MAX_ZK_SIGNING_KEYS {
    if tx_builder.funding_inputs().len() + selected_inputs.len() <= MAX_INPUTS {
        return evaluate_standard_funding_inputs(
            tx_builder,
            selected_inputs,
            change_pk,
            context,
            priority_fee_percent,
        );
    }

    // TODO(RFC-480): UTXO test wallet, a ZkTransfer spends at most four notes.
    // Ok(build_chunked_funded_tx(
    //     tx_builder,
    //     selected_inputs,
    //     change_pk,
    //     context,
    //     priority_fee_percent,
    // )?
    // .map_or(
    //     WalletFundingOutcome::NeedsMoreInputs,
    //     WalletFundingOutcome::Funded,
    // ))
    Ok(WalletFundingOutcome::NeedsMoreInputs)
}

fn evaluate_standard_funding_inputs(
    tx_builder: &MantleTxBuilder,
    selected_inputs: &[OwnedNote],
    change_pk: ZkPublicKey,
    context: &OpsContext,
    priority_fee_percent: u64,
) -> Result<WalletFundingOutcome<MantleTxBuilder>, WalletError> {
    let funded_builder = extend_wallet_funding_inputs(tx_builder, selected_inputs)?;

    match funded_builder
        .funding_delta_with_priority_fee::<MainnetGasProfile>(context, priority_fee_percent)?
        .cmp(&0)
    {
        Ordering::Less => Ok(WalletFundingOutcome::NeedsMoreInputs),
        Ordering::Equal => Ok(WalletFundingOutcome::Funded(funded_builder)),
        // TODO(RFC-480): UTXO test wallet.
        // Ordering::Greater => Ok(funded_builder
        //     .return_change::<MainnetGasProfile>(context, change_pk, priority_fee_percent)?
        Ordering::Greater => Ok(funded_builder
            .return_change::<MainnetGasProfile>(
                context,
                change_pk,
                random_nonce(),
                priority_fee_percent,
            )?
            .map_or(
                WalletFundingOutcome::NeedsMoreInputs,
                WalletFundingOutcome::Funded,
            )),
    }
}

// TODO(RFC-480): UTXO test wallet, a ZkTransfer spends at most four notes.
/*
fn build_chunked_funded_tx(
    tx_builder: &MantleTxBuilder,
    funding_utxos: &[Utxo],
    change_pk: ZkPublicKey,
    context: &OpsContext,
    priority_fee_percent: u64,
) -> Result<Option<MantleTxBuilder>, WalletError> {
    if funding_utxos.len() <= MAX_ZK_SIGNING_KEYS || !tx_builder.ledger_inputs().is_empty() {
        return Ok(None);
    }

    let input_sum = funding_utxos
        .iter()
        .map(|utxo| u128::from(utxo.note.value))
        .sum::<u128>();
    let output_sum = pending_transfer_output_sum(tx_builder);

    let chunked_builder = extend_wallet_funding_inputs(tx_builder, funding_utxos)?;
    let funding_delta = funding_delta_for_chunked_builder(
        &chunked_builder,
        input_sum,
        output_sum,
        context,
        priority_fee_percent,
    )?;

    match funding_delta.cmp(&0) {
        Ordering::Less => Ok(None),
        Ordering::Equal => Ok(Some(chunked_builder)),
        Ordering::Greater => add_chunked_change_output(
            chunked_builder,
            change_pk,
            input_sum,
            output_sum,
            funding_delta,
            context,
            priority_fee_percent,
        ),
    }
}

fn add_chunked_change_output(
    chunked_builder: MantleTxBuilder,
    change_pk: ZkPublicKey,
    input_sum: u128,
    output_sum: u128,
    funding_delta: i128,
    context: &OpsContext,
    priority_fee_percent: u64,
) -> Result<Option<MantleTxBuilder>, WalletError> {
    let builder_with_dummy_change = chunked_builder
        .clone()
        .add_ledger_output(Note::new(0, change_pk))?;
    let delta_with_change = funding_delta_for_chunked_builder(
        &builder_with_dummy_change,
        input_sum,
        output_sum,
        context,
        priority_fee_percent,
    )?;

    if delta_with_change <= 0 {
        return Ok(None);
    }

    let change = u64::try_from(delta_with_change).expect("Positive delta must fit in u64");
    let tx_with_change = chunked_builder.add_ledger_output(Note::new(change, change_pk))?;

    assert_eq!(
        funding_delta_for_chunked_builder(
            &tx_with_change,
            input_sum,
            output_sum + u128::from(change),
            context,
            priority_fee_percent,
        )?,
        0
    );
    debug_assert!(funding_delta > 0);

    Ok(Some(tx_with_change))
}

fn with_transfer_input_chunks(
    tx_builder: &MantleTxBuilder,
    funding_utxos: &[Utxo],
) -> Result<MantleTxBuilder, WalletError> {
    let final_chunk_len = match funding_utxos.len() % MAX_ZK_SIGNING_KEYS {
        0 => MAX_ZK_SIGNING_KEYS,
        remainder => remainder,
    };
    let split_index = funding_utxos.len() - final_chunk_len;

    let mut builder = tx_builder.clone();
    for chunk in funding_utxos[..split_index].chunks(MAX_ZK_SIGNING_KEYS) {
        builder = builder.push_op(Op::Transfer(TransferOp::new(
            Inputs::try_new(chunk.iter().map(Utxo::id).collect::<Vec<_>>())?,
            Outputs::empty(),
        )))?;
    }

    builder
        .extend_ledger_inputs(funding_utxos[split_index..].iter().copied())
        .map_err(Into::into)
}

fn pending_transfer_output_sum(tx_builder: &MantleTxBuilder) -> u128 {
    tx_builder.clone().build().map_or_else(
        |_| 0,
        |tx| match tx.last() {
            Some(Op::Transfer(transfer)) => transfer
                .outputs
                .iter()
                .map(|note| u128::from(note.value))
                .sum(),
            _ => 0,
        },
    )
}

fn funding_delta_for_chunked_builder(
    tx_builder: &MantleTxBuilder,
    input_sum: u128,
    output_sum: u128,
    context: &OpsContext,
    priority_fee_percent: u64,
) -> Result<i128, WalletError> {
    let mandatory_fee = u128::from(
        tx_builder
            .minimum_gas_cost::<MainnetGasProfile>(context)?
            .into_inner(),
    );
    let required_fee = mandatory_fee
        .checked_mul(u128::from(priority_fee_percent))
        .and_then(|priority_fee| priority_fee.checked_add(99))
        .map(|priority_fee| mandatory_fee + priority_fee / 100)
        .ok_or(lb_core::mantle::gas::GasOverflow)?;
    Ok(i128::try_from(input_sum)
        .expect("Input sum must fit in i128")
        .checked_sub(i128::try_from(output_sum).expect("Output sum must fit in i128"))
        .and_then(|delta| {
            delta.checked_sub(i128::try_from(required_fee).expect("Gas fits in i128"))
        })
        .expect("Chunked funding delta must fit in i128"))
}
*/

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use lb_core::mantle::{
        Op,
        note_set::LEDGER_SET,
        notes::NoteNf,
        ops::channel::{
            ChannelId, MsgId,
            inscribe::{ChannelSteps, Inscription, InscriptionOp},
        },
        transactions::{GasPrices, tx_list::ops::OpsGasContext},
    };
    use lb_mmr::MerklePath;
    use lb_testing_framework::configs::wallet::WalletAccount;

    use super::*;
    use crate::common::wallet::WalletFundingSource;

    #[test]
    fn zero_cost_wallet_transaction_still_uses_funding_input() {
        let signing_key = Ed25519Key::from_bytes(&[0u8; 32]);
        let context = OpsContext {
            // TODO(RFC-480): transfer_threshold is removed, holders withdraw
            // their own channel notes.
            // gas_context: OpsGasContext::new(HashMap::new(), HashMap::new(), GasPrices::new(0, 0)),
            // leader_reward_amount: 0,
            gas_context: OpsGasContext::new(HashMap::new(), GasPrices::new(0, 0)),
        };
        let tx_builder = MantleTxBuilder::new()
            .push_op(Op::ChannelInscribe(InscriptionOp {
                channel_id: ChannelId::from([0xAA; 32]),
                inscription: Inscription::new_unchecked(vec![0xab; 1024]),
                parent: MsgId::root(),
                signer: signing_key.public_key().into_unverified(),
                steps: ChannelSteps::default(),
            }))
            .expect("inscription test builder should fit op bounds");
        assert_eq!(
            tx_builder
                .funding_delta::<MainnetGasProfile>(&context)
                .expect("zero-gas inscription funding delta should calculate"),
            0
        );

        let account = WalletAccount::deterministic(1, 2_000_000, false)
            .expect("test wallet account should build");
        // TODO(RFC-480): UTXO test wallet.
        // let funding_utxo = Utxo::new([7u8; 32], 0, Note::new(2_000_000, account.public_key()));
        let funding_utxo = OwnedNote {
            note: Note::new(2_000_000, Fr::from(7u64), account.public_key()),
            note_set: LEDGER_SET,
            nullifier: NoteNf(Fr::from(7u64)),
            path: MerklePath::try_new(0, Vec::new()).expect("an empty path is bounded"),
        };
        let funding_source = WalletFundingSource::new(account, vec![funding_utxo.clone()]);

        let funded_builder = fund_unsponsored_wallet_transaction(
            &tx_builder,
            funding_source.into_funding_utxos(),
            WalletInputSelectionStrategy::LargestFirst,
            &context,
            0,
        )
        .expect("zero-cost wallet transaction should still select a funding input");

        // TODO(RFC-480): UTXO test wallet.
        // assert_eq!(funded_builder.ledger_inputs(), &[funding_utxo]);
        assert_eq!(
            funded_builder.funding_inputs(),
            &[FundingNote {
                note: funding_utxo.note,
                nullifier: funding_utxo.nullifier,
            }]
        );
        assert_eq!(
            funded_builder
                .funding_delta::<MainnetGasProfile>(&context)
                .expect("funded inscription delta should calculate"),
            0
        );

        let funded_tx = funded_builder.build().expect("funded builder should build");
        let Some(Op::Transfer(transfer)) = funded_tx.iter().last() else {
            panic!("wallet funding should leave a transfer op at the end");
        };
        assert!(!transfer.inputs.is_empty());
    }
}
