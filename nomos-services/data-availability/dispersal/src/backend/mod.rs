pub mod kzgrs;

use std::{future::Future, pin::Pin};

use nomos_core::{
    da::{DaDispersal, DaEncoder},
    mantle::{
        ops::channel::{ChannelId, MsgId},
        SignedMantleTx,
    },
};
use overwatch::DynError;
use tokio::sync::oneshot;

use crate::adapters::{network::DispersalNetworkAdapter, wallet::DaWalletAdapter};

pub type DispersalTask = Pin<Box<dyn Future<Output = (ChannelId, Option<SignedMantleTx>)> + Send>>;

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

    async fn process_dispersal(
        &self,
        channel_id: ChannelId,
        parent_msg_id: MsgId,
        data: Vec<u8>,
        sender: oneshot::Sender<Result<Self::BlobId, DynError>>,
    ) -> Result<DispersalTask, DynError>;
}
