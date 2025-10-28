use std::time::Duration;

use ed25519::Signature;
use ed25519_dalek::Signer as _;
use nomos_core::{
    mantle::{
        MantleTx, SignedMantleTx, Transaction as _,
        ledger::Tx as LedgerTx,
        ops::{
            Op, OpProof,
            channel::{
                ChannelId, Ed25519PublicKey, MsgId, blob::BlobOp, inscribe::InscriptionOp,
                set_keys::SetKeysOp,
            },
        },
        tx::TxHash,
    },
    sdp::SessionNumber,
};
use nomos_ledger::mantle::channel::ChannelState;
use tokio::time::{sleep, timeout};
use zksign::SecretKey;

fn empty_ledger_signature(tx_hash: &TxHash) -> zksign::Signature {
    SecretKey::multi_sign(&[], tx_hash.as_ref()).expect("multi-sign with empty key set works")
}

#[must_use]
pub fn create_channel_inscribe_tx(
    signing_key: &ed25519_dalek::SigningKey,
    channel_id: ChannelId,
    inscription: Vec<u8>,
    parent: MsgId,
) -> SignedMantleTx {
    let verifying_key_bytes = signing_key.verifying_key().to_bytes();
    let verifying_key = Ed25519PublicKey::from_bytes(&verifying_key_bytes).unwrap();

    let inscribe_op = InscriptionOp {
        channel_id,
        inscription,
        parent,
        signer: verifying_key,
    };

    sign_single_op_tx(signing_key, Op::ChannelInscribe(inscribe_op))
}

#[must_use]
pub fn create_channel_blob_tx(
    signing_key: &ed25519_dalek::SigningKey,
    channel_id: ChannelId,
    session: SessionNumber,
    blob: [u8; 32],
    blob_size: u64,
    parent: MsgId,
) -> SignedMantleTx {
    let verifying_key_bytes = signing_key.verifying_key().to_bytes();
    let verifying_key = Ed25519PublicKey::from_bytes(&verifying_key_bytes).unwrap();

    let blob_op = BlobOp {
        channel: channel_id,
        session,
        blob,
        blob_size,
        da_storage_gas_price: 0,
        parent,
        signer: verifying_key,
    };

    sign_single_op_tx(signing_key, Op::ChannelBlob(blob_op))
}

#[must_use]
pub fn create_channel_set_keys_tx(
    signing_key: &ed25519_dalek::SigningKey,
    channel_id: ChannelId,
    keys: Vec<Ed25519PublicKey>,
) -> SignedMantleTx {
    let set_keys_op = SetKeysOp {
        channel: channel_id,
        keys,
    };

    sign_single_op_tx(signing_key, Op::ChannelSetKeys(set_keys_op))
}

fn sign_single_op_tx(signing_key: &ed25519_dalek::SigningKey, op: Op) -> SignedMantleTx {
    let mantle_tx = MantleTx {
        ops: vec![op],
        ledger_tx: LedgerTx::new(vec![], vec![]),
        storage_gas_price: 0,
        execution_gas_price: 0,
    };

    let tx_hash = mantle_tx.hash();
    let signature_bytes = signing_key
        .sign(tx_hash.as_signing_bytes().as_ref())
        .to_bytes();
    let signature = Signature::from_bytes(&signature_bytes);

    SignedMantleTx {
        ops_proofs: vec![OpProof::Ed25519Sig(signature)],
        ledger_tx_proof: empty_ledger_signature(&tx_hash),
        mantle_tx,
    }
}

pub async fn fetch_channel_state(base_url: &str, channel_id: ChannelId) -> Option<ChannelState> {
    let request_url = format!(
        "{}mantle/channels/{}",
        base_url,
        hex::encode(channel_id.as_ref()),
    );

    reqwest::get(request_url)
        .await
        .ok()?
        .json::<Option<ChannelState>>()
        .await
        .ok()?
}

pub async fn wait_for_channel_tip_change(
    base_url: &str,
    channel_id: ChannelId,
    previous_tip: MsgId,
    wait_for: Duration,
) -> Option<ChannelState> {
    timeout(wait_for, async {
        loop {
            if let Some(state) = fetch_channel_state(base_url, channel_id).await
                && state.tip != previous_tip
            {
                break state;
            }

            sleep(Duration::from_secs(1)).await;
        }
    })
    .await
    .ok()
}
