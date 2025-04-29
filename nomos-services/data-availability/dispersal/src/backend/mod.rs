use std::{fmt::Debug, time::Duration};

use nomos_core::da::{blob::metadata, DaDispersal, DaEncoder};
use nomos_tracing::info_with_id;
use overwatch::DynError;
use tracing::instrument;

use crate::adapters::{mempool::DaMempoolAdapter, network::DispersalNetworkAdapter};

pub mod kzgrs;

#[async_trait::async_trait]
pub trait DispersalBackend {
    type Settings;
    type Encoder: DaEncoder;
    type Dispersal: DaDispersal<EncodedData = <Self::Encoder as DaEncoder>::EncodedData>;
    type NetworkAdapter: DispersalNetworkAdapter;
    type MempoolAdapter: DaMempoolAdapter;
    type Metadata: Debug + metadata::Metadata + Send;
    type BlobId: AsRef<[u8]> + Send + Copy;

    fn init(
        config: Self::Settings,
        network_adapter: Self::NetworkAdapter,
        mempool_adapter: Self::MempoolAdapter,
    ) -> Self;
    async fn encode(
        &self,
        data: Vec<u8>,
    ) -> Result<(Self::BlobId, <Self::Encoder as DaEncoder>::EncodedData), DynError>;
    async fn disperse(
        &self,
        encoded_data: <Self::Encoder as DaEncoder>::EncodedData,
    ) -> Result<(), DynError>;

    async fn publish_to_mempool(
        &self,
        blob_id: Self::BlobId,
        metadata: Self::Metadata,
    ) -> Result<(), DynError>;

    #[instrument(skip_all)]
    async fn process_dispersal(
        &self,
        data: Vec<u8>,
        metadata: Self::Metadata,
    ) -> Result<Self::BlobId, DynError> {
        let (blob_id, encoded_data) = self.encode(data).await?;
        info_with_id!(blob_id.as_ref(), "ProcessDispersal");
        self.disperse(encoded_data).await?;
        // let disperse and replication happen before pushing to mempool
        tokio::time::sleep(Duration::from_secs(1)).await;
        self.publish_to_mempool(blob_id, metadata).await?;
        Ok(blob_id)
    }
}
