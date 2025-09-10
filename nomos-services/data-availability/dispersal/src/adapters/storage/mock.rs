use nomos_core::mantle::SignedMantleTx;

use super::DispersalStorageAdapter;

pub struct MockDispersalStorageAdapter {
    last_tx: Option<SignedMantleTx>,
}

impl DispersalStorageAdapter for MockDispersalStorageAdapter {
    fn new() -> Self {
        Self { last_tx: None }
    }

    fn last_tx(&self) -> Option<SignedMantleTx> {
        self.last_tx.clone()
    }

    fn store_tx(&mut self, tx: SignedMantleTx) -> Result<(), super::DispersalStorageError> {
        self.last_tx = Some(tx);
        Ok(())
    }
}
