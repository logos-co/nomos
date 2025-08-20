use nomos_core::da::{DaDispersal, DaEncoder};
use overwatch::DynError;

use crate::adapters::{network::DispersalNetworkAdapter, wallet::DaWalletAdapter};

pub mod kzgrs;

#[async_trait::async_trait]
pub trait DispersalBackend {
    type Settings;
    type Encoder: DaEncoder;
    type Dispersal: DaDispersal<EncodedData = <Self::Encoder as DaEncoder>::EncodedData>;
    type NetworkAdapter: DispersalNetworkAdapter;
    type WalletAdapter: DaWalletAdapter;
    type BlobId: AsRef<[u8]> + Send + Copy;

    fn init(
        config: Self::Settings,
        network_adapter: Self::NetworkAdapter,
        wallet_adapter: Self::WalletAdapter,
    ) -> Self;

    async fn process_dispersal(&self, data: Vec<u8>) -> Result<Self::BlobId, DynError>;
}
