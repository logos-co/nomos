use std::convert::Infallible;

use ed25519::signature::Signer as _;
use ed25519_dalek::SigningKey;
use nomos_core::{
    mantle::{
        Op, OpProof, SignedMantleTx, Transaction as _, ops::channel::blob::BlobOp,
        tx_builder::MantleTxBuilder,
    },
    proofs::zksig::{DummyZkSignature, ZkSignaturePublic},
};

use super::{BlobOpArgs, DaWalletAdapter};

pub struct MockWalletAdapter;

impl DaWalletAdapter for MockWalletAdapter {
    type Error = Infallible;

    fn new() -> Self {
        Self {}
    }

    fn blob_tx(
        &self,
        tx_builder: MantleTxBuilder,
        blob_op_args: BlobOpArgs,
    ) -> Result<SignedMantleTx, Self::Error> {
        // TODO: This mock implementation targets to only work with integration tests.
        // When integration tests genesis_state changes, this part should be updated, or
        // removed all together after an actual wallet service can create signed mantle
        // transaction with blob operation.
        let BlobOpArgs {
            channel_id,
            current_session,
            parent_msg_id,
            blob_id,
            blob_size,
            ..
        } = blob_op_args;

        // Hardcoded signing key for testing (matches the all-zeros key expected in
        // tests) TODO: In production, this should come from a key management
        // system
        let signing_key = SigningKey::from_bytes(&[0u8; 32]);
        let signer = signing_key.verifying_key();

        let blob_op = BlobOp {
            channel: channel_id,
            current_session,
            blob: blob_id,
            blob_size: blob_size as u64,
            da_storage_gas_price: 3000,
            parent: parent_msg_id,
            signer,
        };

        // TODO: Wallet service will expect a BlobOp, and transform that into a
        // SignedMantleTx.
        let mantle_tx = tx_builder.push_op(Op::ChannelBlob(blob_op)).build();

        // Sign the transaction hash
        let tx_hash = mantle_tx.hash();
        let signature = signing_key.sign(&tx_hash.as_signing_bytes());

        // Create signed transaction with valid signature proof
        Ok(SignedMantleTx::new(
            mantle_tx,
            vec![OpProof::Ed25519Sig(signature)],
            DummyZkSignature::prove(&ZkSignaturePublic {
                msg_hash: tx_hash.into(),
                pks: vec![],
            }),
        )
        .expect("Transaction with valid signature should be valid"))
    }
}
