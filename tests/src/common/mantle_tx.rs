use ed25519::Signature;
use ed25519_dalek::{Signer as _, ed25519};
use groth16::CompressedGroth16Proof;
use nomos_core::{
    mantle::{
        MantleTx, Note, NoteId, SignedMantleTx, Transaction as _,
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
    sdp::{
        ActiveMessage, ActivityMetadata, DaActivityProof, DeclarationMessage, ServiceType,
        SessionNumber, WithdrawMessage,
    },
};
use zksign::PublicKey;

fn empty_zk_signature() -> zksign::Signature {
    zksign::Signature::new(CompressedGroth16Proof::from_bytes(&[0u8; 128]))
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
    let signature = Signature::from_bytes(&signature_bytes);

    SignedMantleTx {
        ops_proofs: vec![OpProof::Ed25519Sig(signature)],
        ledger_tx_proof: empty_zk_signature(),
        mantle_tx: inscribe_tx,
    }
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
    let signature = Signature::from_bytes(&signature_bytes);

    SignedMantleTx {
        ops_proofs: vec![OpProof::Ed25519Sig(signature)],
        ledger_tx_proof: empty_zk_signature(),
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
    let signature = Signature::from_bytes(&signature_bytes);

    SignedMantleTx {
        ops_proofs: vec![OpProof::Ed25519Sig(signature)],
        ledger_tx_proof: empty_zk_signature(),
        mantle_tx: set_keys_tx,
    }
}

#[must_use]
pub fn create_sdp_declare_tx(
    provider_signing_key: &ed25519_dalek::SigningKey,
    service_type: ServiceType,
    locators: Vec<nomos_core::sdp::Locator>,
    zk_id: PublicKey,
    locked_note_id: NoteId,
    note: Note,
) -> (SignedMantleTx, DeclarationMessage) {
    let provider_pk_bytes = provider_signing_key.verifying_key().to_bytes();
    let provider_id = nomos_core::sdp::ProviderId::try_from(provider_pk_bytes)
        .expect("Valid provider id from signing key");

    let declaration = DeclarationMessage {
        service_type,
        locators,
        provider_id,
        zk_id,
        locked_note_id,
    };

    let mantle_tx = MantleTx {
        ops: vec![Op::SDPDeclare(declaration.clone())],
        ledger_tx: LedgerTx::new(vec![], vec![]),
        execution_gas_price: 0,
        storage_gas_price: 0,
    };

    let tx_hash = mantle_tx.hash();

    let ed25519_signature_bytes = provider_signing_key
        .sign(tx_hash.as_signing_bytes().as_ref())
        .to_bytes();
    let ed25519_sig = Signature::from_bytes(&ed25519_signature_bytes);

    let zk_sig = empty_zk_signature();

    let signed_tx = SignedMantleTx {
        ops_proofs: vec![OpProof::ZkAndEd25519Sigs {
            zk_sig,
            ed25519_sig,
        }],
        ledger_tx_proof: empty_zk_signature(),
        mantle_tx,
    };

    (signed_tx, declaration)
}

#[must_use]
pub fn create_sdp_active_tx(active: ActiveMessage, zk_id: PublicKey, note: Note) -> SignedMantleTx {
    let mantle_tx = MantleTx {
        ops: vec![Op::SDPActive(active)],
        ledger_tx: LedgerTx::new(vec![], vec![]),
        execution_gas_price: 0,
        storage_gas_price: 0,
    };

    let tx_hash = mantle_tx.hash();
    let zk_sig = empty_zk_signature();

    SignedMantleTx {
        ops_proofs: vec![OpProof::ZkSig(zk_sig)],
        ledger_tx_proof: empty_zk_signature(),
        mantle_tx,
    }
}

#[must_use]
pub fn create_sdp_withdraw_tx(
    withdraw: WithdrawMessage,
    zk_id: PublicKey,
    note: Note,
) -> SignedMantleTx {
    let mantle_tx = MantleTx {
        ops: vec![Op::SDPWithdraw(withdraw)],
        ledger_tx: LedgerTx::new(vec![], vec![]),
        execution_gas_price: 0,
        storage_gas_price: 0,
    };

    let tx_hash = mantle_tx.hash();
    let zk_sig = empty_zk_signature();

    SignedMantleTx {
        ops_proofs: vec![OpProof::ZkSig(zk_sig)],
        ledger_tx_proof: empty_zk_signature(),
        mantle_tx,
    }
}

#[must_use]
pub const fn empty_da_activity_proof() -> ActivityMetadata {
    ActivityMetadata::DataAvailability(DaActivityProof {
        current_session: 0,
        previous_session_opinions: vec![],
        current_session_opinions: vec![],
    })
}
