use ed25519_dalek::{Signer as _, ed25519};
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
    },
    proofs::zksig::{DummyZkSignature, ZkSignaturePublic},
};

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

    // Use empty ledger tx for zero gas transactions
    let ledger_tx = LedgerTx::new(vec![], vec![]);

    let inscribe_tx = MantleTx {
        ops: vec![Op::ChannelInscribe(inscribe_op)],
        ledger_tx,
        storage_gas_price: 0,
        execution_gas_price: 0,
    };

    let tx_hash = inscribe_tx.hash();
    let signature_bytes = signing_key
        .sign(tx_hash.as_signing_bytes().as_ref())
        .to_bytes();
    let signature = ed25519::Signature::from_bytes(&signature_bytes);

    SignedMantleTx {
        ops_proofs: vec![OpProof::Ed25519Sig(signature)],
        ledger_tx_proof: DummyZkSignature::prove(&ZkSignaturePublic {
            msg_hash: tx_hash.into(),
            pks: vec![],
        }),
        mantle_tx: inscribe_tx,
    }
}

#[must_use]
pub fn create_channel_blob_tx(
    signing_key: &ed25519_dalek::SigningKey,
    channel_id: ChannelId,
    blob: [u8; 32],
    blob_size: u64,
    parent: MsgId,
) -> SignedMantleTx {
    let verifying_key_bytes = signing_key.verifying_key().to_bytes();
    let verifying_key = Ed25519PublicKey::from_bytes(&verifying_key_bytes).unwrap();

    let blob_op = BlobOp {
        channel: channel_id,
        blob,
        blob_size,
        da_storage_gas_price: 0,
        parent,
        signer: verifying_key,
    };

    let blob_tx = MantleTx {
        ops: vec![Op::ChannelBlob(blob_op)],
        ledger_tx: LedgerTx::new(vec![], vec![]),
        storage_gas_price: 0,
        execution_gas_price: 0,
    };

    let tx_hash = blob_tx.hash();
    let signature_bytes = signing_key
        .sign(tx_hash.as_signing_bytes().as_ref())
        .to_bytes();
    let signature = ed25519::Signature::from_bytes(&signature_bytes);

    SignedMantleTx {
        ops_proofs: vec![OpProof::Ed25519Sig(signature)],
        ledger_tx_proof: DummyZkSignature::prove(&ZkSignaturePublic {
            msg_hash: tx_hash.into(),
            pks: vec![],
        }),
        mantle_tx: blob_tx,
    }
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

    let set_keys_tx = MantleTx {
        ops: vec![Op::ChannelSetKeys(set_keys_op)],
        ledger_tx: LedgerTx::new(vec![], vec![]),
        storage_gas_price: 0,
        execution_gas_price: 0,
    };

    let tx_hash = set_keys_tx.hash();
    let signature_bytes = signing_key
        .sign(tx_hash.as_signing_bytes().as_ref())
        .to_bytes();
    let signature = ed25519::Signature::from_bytes(&signature_bytes);

    SignedMantleTx {
        ops_proofs: vec![OpProof::Ed25519Sig(signature)],
        ledger_tx_proof: DummyZkSignature::prove(&ZkSignaturePublic {
            msg_hash: tx_hash.into(),
            pks: vec![],
        }),
        mantle_tx: set_keys_tx,
    }
}
