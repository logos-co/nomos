use async_trait::async_trait;
use nomos_core::{header::HeaderId, mantle::TxHash};

pub mod adapter;

#[async_trait]
pub trait MempoolAdapter<Tx>: Send + Sync {
    async fn mark_transactions_in_block(
        &self,
        ids: &[TxHash],
        block: HeaderId,
    ) -> Result<(), overwatch::DynError>;
}
