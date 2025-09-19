use std::convert::Infallible;

use nomos_core::{
    da::BlobId,
    mantle::{
        ledger::Tx as LedgerTx,
        ops::channel::{blob::BlobOp, ChannelId, Ed25519PublicKey, MsgId},
        MantleTx, Note, Op, SignedMantleTx, Transaction as _, Utxo,
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
        signer: Ed25519PublicKey,
    ) -> Result<SignedMantleTx, Self::Error> {
        // TODO: This mock implementation targets to only work with integration tests.
        // When integration tests genesis_state changes, this part should be updated, or
        // removed all together after an actual wallet service can create signed mantle
        // transaction with blob operation.
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

        Ok(SignedMantleTx {
            ops_proofs: vec![None],
            ledger_tx_proof: DummyZkSignature::prove(ZkSignaturePublic {
                msg_hash: mantle_tx.hash().into(),
                pks: vec![],
            }),
            mantle_tx,
        })
    }
}
