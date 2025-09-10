pub mod mock;

use nomos_core::mantle::SignedMantleTx;

pub struct DispersalStorageError;

pub trait DispersalStorageAdapter {
    fn new() -> Self;
    fn last_tx(&self) -> Option<SignedMantleTx>;
    fn store_tx(&mut self, tx: SignedMantleTx) -> Result<(), DispersalStorageError>;
}
