use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use nomos_core::header::HeaderId;

pub mod adapter;

#[async_trait]
pub trait MempoolAdapter<Tx>: Send + Sync {
    async fn get_mempool_view(
        &self,
        ancestor_hint: HeaderId,
    ) -> Result<Pin<Box<dyn Stream<Item = Tx> + Send>>, overwatch::DynError>;
}
