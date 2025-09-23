use std::fmt::{Debug, Display};

use chain_service::{
    ConsensusMsg, CryptarchiaConsensus, CryptarchiaInfo,
    network::adapters::libp2p::LibP2pAdapter as ConsensusNetworkAdapter,
};
use nomos_core::{
    da::BlobId,
    header::HeaderId,
    mantle::{SignedMantleTx, Transaction},
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_storage::backends::rocksdb::RocksBackend;
use overwatch::{overwatch::handle::OverwatchHandle, services::AsServiceId};
use tokio::sync::oneshot;
use tx_service::{
    backend::Mempool, network::adapters::libp2p::Libp2pAdapter as MempoolNetworkAdapter,
};

use crate::http::DynError;

pub type MempoolBackend<StorageAdapter, RuntimeServiceId> = Mempool<
    HeaderId,
    SignedMantleTx,
    <SignedMantleTx as Transaction>::Hash,
    StorageAdapter,
    RuntimeServiceId,
>;

pub type Cryptarchia<
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    StorageAdapter,
    TimeBackend,
    RuntimeServiceId,
> = CryptarchiaConsensus<
    ConsensusNetworkAdapter<SignedMantleTx, RuntimeServiceId>,
    MempoolBackend<StorageAdapter, RuntimeServiceId>,
    MempoolNetworkAdapter<SignedMantleTx, <SignedMantleTx as Transaction>::Hash, RuntimeServiceId>,
    RocksBackend,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
>;

pub async fn cryptarchia_info<
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    StorageAdapter,
    TimeBackend,
    RuntimeServiceId,
>(
    handle: &OverwatchHandle<RuntimeServiceId>,
) -> Result<CryptarchiaInfo, DynError>
where
    SamplingBackend: DaSamplingServiceBackend<BlobId = BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    StorageAdapter: tx_service::storage::MempoolStorageAdapter<
            RuntimeServiceId,
            Key = <SignedMantleTx as Transaction>::Hash,
            Item = SignedMantleTx,
        > + Clone
        + 'static,
    StorageAdapter::Error: Debug,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<
            Cryptarchia<
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                StorageAdapter,
                TimeBackend,
                RuntimeServiceId,
            >,
        >,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(ConsensusMsg::Info { tx: sender })
        .await
        .map_err(|(e, _)| e)?;

    Ok(receiver.await?)
}

pub async fn cryptarchia_headers<
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    StorageAdapter,
    TimeBackend,
    RuntimeServiceId,
>(
    handle: &OverwatchHandle<RuntimeServiceId>,
    from: Option<HeaderId>,
    to: Option<HeaderId>,
) -> Result<Vec<HeaderId>, DynError>
where
    SamplingBackend: DaSamplingServiceBackend<BlobId = BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    StorageAdapter: tx_service::storage::MempoolStorageAdapter<
            RuntimeServiceId,
            Key = <SignedMantleTx as Transaction>::Hash,
            Item = SignedMantleTx,
        > + Clone
        + 'static,
    StorageAdapter::Error: Debug,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<
            Cryptarchia<
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                StorageAdapter,
                TimeBackend,
                RuntimeServiceId,
            >,
        >,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(ConsensusMsg::GetHeaders {
            from,
            to,
            tx: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    Ok(receiver.await?)
}
