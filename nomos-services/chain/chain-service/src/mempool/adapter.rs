use std::marker::PhantomData;

use nomos_core::{header::HeaderId, mantle::TxHash};
use nomos_mempool::MempoolMsg;
use overwatch::services::relay::OutboundRelay;
use tokio::sync::oneshot;

#[async_trait::async_trait]
pub trait MempoolAdapterTrait<Tx> {
    async fn get_transactions(&self, hashes: &[TxHash]) -> Result<Vec<Tx>, overwatch::DynError>;
}

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
    async fn get_transactions(&self, hashes: &[TxHash]) -> Result<Vec<Tx>, overwatch::DynError> {
        let (reply_channel, receiver) = oneshot::channel();

        self.mempool_relay
            .send(MempoolMsg::GetTransactionsByHashes {
                hashes: hashes.to_vec(),
                reply_channel,
            })
            .await
            .map_err(|(e, _)| format!("Could not get transactions from mempool: {e}"))?;

        let transactions = receiver
            .await
            .map_err(|e| format!("Failed to get transactions: {e}"))?;

        Ok(transactions)
    }
}
