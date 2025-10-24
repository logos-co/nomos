use nomos_core::mantle::{SignedMantleTx, Transaction as _};
use overwatch::DynError;

use super::SdpMempoolAdapter;

pub struct MockMempoolAdapter;

#[async_trait::async_trait]
impl SdpMempoolAdapter for MockMempoolAdapter {
    fn new() -> Self {
        Self {}
    }

    async fn post_tx(&self, tx: SignedMantleTx) -> Result<(), DynError> {
        // TODO: enable when membership service is deleted and we can depend on
        // tx-service
        //     self.mempool_relay
        //      .send(MempoolMsg::Add {
        //     key: tx.mantle_tx.hash(),
        //     payload: tx,
        //     reply_channel,
        // })
        tracing::debug!("Mock: posting tx with hash {:?}", tx.hash());
        Ok(())
    }
}
