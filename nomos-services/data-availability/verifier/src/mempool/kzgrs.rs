use std::{fmt::Debug, marker::PhantomData};

use nomos_core::{
    header::HeaderId,
    mantle::{SignedMantleTx, Transaction as _, TxHash},
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_mempool::{
    MempoolMsg, TxMempoolService,
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as MempoolAdapter,
};
use overwatch::services::{ServiceData, relay::OutboundRelay};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use super::{DaMempoolAdapter, MempoolAdapterError};

type MempoolRelay<Item, Key> = OutboundRelay<MempoolMsg<HeaderId, Item, Item, Key>>;

pub struct KzgrsMempoolAdapter<
    ClPoolAdapter,
    ClPool,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    RuntimeServiceId,
> where
    ClPool: MemPool<BlockId = HeaderId>,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = ClPool::Key>,
    ClPool::Item: Clone + Eq + Debug + 'static,
    ClPool::Key: Debug + 'static,
{
    pub mempool_relay: MempoolRelay<ClPool::Item, ClPool::Key>,
    _phantom: PhantomData<(
        ClPoolAdapter,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        RuntimeServiceId,
    )>,
}

#[async_trait::async_trait]
impl<
    ClPoolAdapter,
    ClPool,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    RuntimeServiceId,
> DaMempoolAdapter
    for KzgrsMempoolAdapter<
        ClPoolAdapter,
        ClPool,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        RuntimeServiceId,
    >
where
    ClPool: RecoverableMempool<BlockId = HeaderId, Key = TxHash, Item = SignedMantleTx>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone,
    ClPoolAdapter:
        MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key> + Sync,
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
        ClPoolAdapter,
        SamplingNetworkAdapter,
        SamplingStorage,
        ClPool,
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
