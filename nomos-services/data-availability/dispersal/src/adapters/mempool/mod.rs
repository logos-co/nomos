pub mod kzgrs;

use std::error::Error;

use nomos_mempool::backend::MempoolError;
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};

#[derive(Debug)]
pub enum DaMempoolAdapterError {
    Mempool(MempoolError),
    Other(DynError),
}

impl<E> From<E> for DaMempoolAdapterError
where
    E: Error + Send + Sync + 'static,
{
    fn from(e: E) -> Self {
        Self::Other(Box::new(e) as DynError)
    }
}

#[async_trait::async_trait]
pub trait DaMempoolAdapter {
    type MempoolService: ServiceData;
    type BlobId;
    type Metadata;

    fn new(outbound_relay: OutboundRelay<<Self::MempoolService as ServiceData>::Message>) -> Self;

    async fn post_blob_id(
        &self,
        blob_id: Self::BlobId,
        metadata: Self::Metadata,
    ) -> Result<(), DaMempoolAdapterError>;
}
