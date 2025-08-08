use std::convert::Infallible;

use nomos_core::{
    da::BlobId,
    mantle::{
        ledger::Tx as LedgerTx,
        ops::{blob::BlobOp, Ed25519PublicKey, Op},
        MantleTx, SignedMantleTx,
    },
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
            channel: 0,
            blob,
            blob_size: blob_size as u64,
            da_storage_gas_price: 0,
            after_tx: None,
            signer,
        };

        let mantle_tx = MantleTx {
            ops: vec![Op::Blob(blob_op)],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            storage_gas_price: 0,
            execution_gas_price: 0,
        };

        Ok(SignedMantleTx {
            mantle_tx,
            ops_profs: Vec::new(),
            ledger_tx_proof: (),
        })
    }
}
