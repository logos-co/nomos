pub mod mock;

use nomos_core::mantle::SignedMantleTx;
use overwatch::DynError;

#[async_trait::async_trait]
pub trait SdpMempoolAdapter {
    fn new() -> Self;

    async fn post_tx(&self, tx: SignedMantleTx) -> Result<(), DynError>;
}
