use std::{fmt::Debug, marker::PhantomData};

use nomos_core::{
    header::HeaderId,
    mantle::{SignedMantleTx, Transaction as _, TxHash},
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use overwatch::services::{ServiceData, relay::OutboundRelay};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tx_service::{MempoolMsg, TxMempoolService, backend::RecoverableMempool};

use super::{DaMempoolAdapter, MempoolAdapterError};

type MempoolRelay<Item, Key> = OutboundRelay<MempoolMsg<HeaderId, Item, Item, Key>>;

pub struct KzgrsMempoolAdapter<
    MempoolNetAdapter,
    Mempool,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    RuntimeServiceId,
> where
    Mempool: tx_service::backend::Mempool<BlockId = HeaderId>,
    MempoolNetAdapter: tx_service::network::NetworkAdapter<RuntimeServiceId, Key = Mempool::Key>,
    Mempool::Item: Clone + Eq + Debug + 'static,
    Mempool::Key: Debug + 'static,
{
    pub mempool_relay: MempoolRelay<Mempool::Item, Mempool::Key>,
    _phantom: PhantomData<(
        MempoolNetAdapter,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        RuntimeServiceId,
    )>,
}

#[async_trait::async_trait]
impl<
    MempoolNetAdapter,
    Mempool,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    RuntimeServiceId,
> DaMempoolAdapter
    for KzgrsMempoolAdapter<
        MempoolNetAdapter,
        Mempool,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        RuntimeServiceId,
    >
where
    Mempool: RecoverableMempool<BlockId = HeaderId, Key = TxHash, Item = SignedMantleTx>,
    Mempool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    Mempool::Settings: Clone,
    MempoolNetAdapter:
        tx_service::network::NetworkAdapter<RuntimeServiceId, Key = Mempool::Key> + Sync,
    SamplingBackend: DaSamplingServiceBackend + Send + Sync,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    RuntimeServiceId: Send + Sync,
{
    type MempoolService = TxMempoolService<
        MempoolNetAdapter,
        SamplingNetworkAdapter,
        SamplingStorage,
        Mempool,
        RuntimeServiceId,
    >;
    type Tx = SignedMantleTx;

    fn new(mempool_relay: OutboundRelay<<Self::MempoolService as ServiceData>::Message>) -> Self {
        Self {
            mempool_relay,
            _phantom: PhantomData,
        }
    }

    async fn post_tx(&self, tx: Self::Tx) -> Result<(), MempoolAdapterError> {
        let (reply_channel, receiver) = oneshot::channel();
        self.mempool_relay
            .send(MempoolMsg::Add {
                key: tx.hash(),
                payload: tx,
                reply_channel,
            })
            .await
            .map_err(|(e, _)| MempoolAdapterError::Other(Box::new(e)))?;

        receiver.await?.map_err(MempoolAdapterError::Mempool)
    }
}
