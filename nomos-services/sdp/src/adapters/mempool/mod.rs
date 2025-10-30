use nomos_core::mantle::SignedMantleTx;
use overwatch::{DynError, overwatch::OverwatchHandle};

#[async_trait::async_trait]
pub trait SdpMempoolAdapter<RuntimeServiceId>: Sized {
    async fn new(overwatch_handle: &OverwatchHandle<RuntimeServiceId>) -> Result<Self, DynError>;

    async fn post_tx(&self, tx: SignedMantleTx) -> Result<(), DynError>;
}
