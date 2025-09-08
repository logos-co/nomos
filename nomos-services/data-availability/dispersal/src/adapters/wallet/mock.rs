use std::convert::Infallible;

use nomos_core::{
    da::BlobId,
    mantle::{
        ledger::Tx as LedgerTx,
        ops::channel::{blob::BlobOp, ChannelId, Ed25519PublicKey, MsgId},
        MantleTx, Op, SignedMantleTx, Transaction as _,
    },
    proofs::zksig::{DummyZkSignature, ZkSignaturePublic},
};

use super::DaWalletAdapter;

pub struct MockWalletAdapter;

impl DaWalletAdapter for MockWalletAdapter {
    type Error = Infallible;

    fn new() -> Self {
        Self
    }

    fn blob_tx(
        &self,
        blob: BlobId,
        blob_size: usize,
        signer: Ed25519PublicKey,
    ) -> Result<SignedMantleTx, Self::Error> {
        let blob_op = BlobOp {
            channel: ChannelId::from([0; 32]),
            blob,
            blob_size: blob_size as u64,
            da_storage_gas_price: 0,
            parent: MsgId::root(),
            signer,
        };

        let mantle_tx = MantleTx {
            ops: vec![Op::ChannelBlob(blob_op)],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            storage_gas_price: 0,
            execution_gas_price: 0,
        };

        Ok(SignedMantleTx {
            ops_proofs: Vec::new(),
            ledger_tx_proof: DummyZkSignature::prove(ZkSignaturePublic {
                msg_hash: mantle_tx.hash().into(),
                pks: vec![],
            }),
            mantle_tx,
        })
    }
}
