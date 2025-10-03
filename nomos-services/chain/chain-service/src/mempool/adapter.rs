use std::marker::PhantomData;

use nomos_core::{header::HeaderId, mantle::TxHash};
use overwatch::services::relay::OutboundRelay;
use tx_service::MempoolMsg;

use super::MempoolAdapter as MempoolAdapterTrait;

pub struct MempoolAdapter<Payload, Tx> {
    mempool_relay: OutboundRelay<MempoolMsg<HeaderId, Payload, Tx, TxHash>>,
    _payload: PhantomData<Payload>,
}

impl<Payload, Tx> MempoolAdapter<Payload, Tx> {
    #[must_use]
    pub const fn new(
        mempool_relay: OutboundRelay<MempoolMsg<HeaderId, Payload, Tx, TxHash>>,
    ) -> Self {
        Self {
            mempool_relay,
            _payload: PhantomData,
        }
    }
}

#[async_trait::async_trait]
impl<Payload, Tx> MempoolAdapterTrait<Tx> for MempoolAdapter<Payload, Tx>
where
    Payload: Send + Sync,
    Tx: Send + Sync + 'static,
{
    async fn mark_transactions_in_block(
        &self,
        ids: &[TxHash],
        block: HeaderId,
    ) -> Result<(), overwatch::DynError> {
        self.mempool_relay
            .send(MempoolMsg::MarkInBlock {
                ids: ids.to_vec(),
                block,
            })
            .await
            .map_err(|(e, _)| format!("Could not mark transactions in block: {e}"))?;

        Ok(())
    }
}
