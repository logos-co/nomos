use std::fmt::Debug;

use nomos_core::da::{blob::metadata, DaDispersal, DaEncoder};
use overwatch::DynError;

use crate::adapters::{
    mempool::DaMempoolAdapter, network::DispersalNetworkAdapter, wallet::DaWalletAdapter,
};

pub mod kzgrs;

#[async_trait::async_trait]
pub trait DispersalBackend {
    type Settings;
    type Encoder: DaEncoder;
    type Dispersal: DaDispersal<EncodedData = <Self::Encoder as DaEncoder>::EncodedData>;
    type NetworkAdapter: DispersalNetworkAdapter;
    type MempoolAdapter: DaMempoolAdapter;
    type WalletAdapter: DaWalletAdapter;
    type Metadata: Debug + metadata::Metadata + Send;
    type BlobId: AsRef<[u8]> + Send + Copy;

    fn init(
        config: Self::Settings,
        network_adapter: Self::NetworkAdapter,
        mempool_adapter: Self::MempoolAdapter,
        wallet_adapter: Self::WalletAdapter,
    ) -> Self;

    async fn process_dispersal(
        &self,
        data: Vec<u8>,
        metadata: Self::Metadata,
    ) -> Result<Self::BlobId, DynError>;
}
