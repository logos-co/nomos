use std::convert::Infallible;

use ed25519::signature::Signer as _;
use ed25519_dalek::SigningKey;
use nomos_core::{
    da::BlobId,
    mantle::{
        MantleTx, Note, Op, OpProof, SignedMantleTx, Transaction as _, Utxo,
        ledger::Tx as LedgerTx,
        ops::channel::{ChannelId, MsgId, blob::BlobOp},
    },
    proofs::zksig::{DummyZkSignature, ZkSignaturePublic},
};
use num_bigint::BigUint;

use super::DaWalletAdapter;

pub struct MockWalletAdapter;

impl DaWalletAdapter for MockWalletAdapter {
    type Error = Infallible;

    fn new() -> Self {
        Self {}
    }

    fn blob_tx(
        &self,
        channel_id: ChannelId,
        parent_msg_id: MsgId,
        blob: BlobId,
        blob_size: usize,
    ) -> Result<SignedMantleTx, Self::Error> {
        // TODO: This mock implementation targets to only work with integration tests.
        // When integration tests genesis_state changes, this part should be updated, or
        // removed all together after an actual wallet service can create signed mantle
        // transaction with blob operation.

        // Hardcoded signing key for testing (matches the all-zeros key expected in
        // tests) TODO: In production, this should come from a key management
        // system
        let signing_key = SigningKey::from_bytes(&[0u8; 32]);
        let signer = signing_key.verifying_key();

        let utxo = Utxo {
            note: Note::new(1, BigUint::from(0u8).into()),
            tx_hash: BigUint::from(0u8).into(),
            output_index: 0,
        };

        let blob_op = BlobOp {
            channel: channel_id,
            blob,
            blob_size: blob_size as u64,
            da_storage_gas_price: 3000,
            parent: parent_msg_id,
            signer,
        };

        let mantle_tx = MantleTx {
            ops: vec![Op::ChannelBlob(blob_op)],
            ledger_tx: LedgerTx::new(vec![utxo.id()], vec![]),
            storage_gas_price: 3000,
            execution_gas_price: 3000,
        };

        // Sign the transaction hash
        let tx_hash = mantle_tx.hash();
        let signature = signing_key.sign(&tx_hash.as_signing_bytes());

        // Create signed transaction with valid signature proof
        Ok(SignedMantleTx::new(
            mantle_tx,
            vec![Some(OpProof::Ed25519Sig(signature))],
            DummyZkSignature::prove(ZkSignaturePublic {
                msg_hash: tx_hash.into(),
                pks: vec![],
            }),
        )
        .expect("Transaction with valid signature should be valid"))
    }
}
