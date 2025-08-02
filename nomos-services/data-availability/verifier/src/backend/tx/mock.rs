use nomos_core::mantle::SignedMantleTx;

use crate::backend::TxVerifierBackend;

pub struct MockTxVerifier;

impl TxVerifierBackend for MockTxVerifier {
    type Settings = ();
    type Tx = SignedMantleTx;
    type Error = ();

    fn new(_settings: Self::Settings) -> Self {
        Self
    }

    fn verify(&self, _tx: Self::Tx) -> Result<(), Self::Error> {
        Ok(())
    }
}
