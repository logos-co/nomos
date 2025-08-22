use std::{fmt::Debug, marker::PhantomData};

use kzgrs_backend::dispersal::{BlobInfo, Index, Metadata};
use nomos_core::{
    da::{blob::info::DispersedBlobInfo, BlobId},
    header::HeaderId,
    mantle::SignedMantleTx,
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_mempool::{
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as MempoolAdapter,
    DaMempoolService, MempoolMsg,
};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use super::{DaMempoolAdapter, MempoolAdapterError};

type MempoolRelay<Payload, Item, Key> = OutboundRelay<MempoolMsg<HeaderId, Payload, Item, Key>>;

pub struct KzgrsMempoolAdapter<
    DaPoolAdapter,
    DaPool,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    RuntimeServiceId,
> where
    DaPool: MemPool<BlockId = HeaderId>,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    DaPool::Item: Clone + Eq + Debug + 'static,
    DaPool::Key: Debug + 'static,
{
    pub mempool_relay: MempoolRelay<DaPoolAdapter::Payload, DaPool::Item, DaPool::Key>,
    _phantom: PhantomData<(SamplingBackend, SamplingNetworkAdapter, SamplingStorage)>,
}

#[async_trait::async_trait]
impl<
        DaPoolAdapter,
        DaPool,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        RuntimeServiceId,
    > DaMempoolAdapter
    for KzgrsMempoolAdapter<
        DaPoolAdapter,
        DaPool,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        RuntimeServiceId,
    >
where
    DaPool: RecoverableMempool<BlockId = HeaderId, Key = BlobId>,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key, Payload = BlobInfo>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug + Send,
    DaPool::Item: Clone + Eq + Debug + Send + 'static,
    DaPool::Key: Debug + Send + 'static,
    DaPool::Settings: Clone,
    SamplingBackend: DaSamplingServiceBackend<BlobId = DaPool::Key> + Send + Sync,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
{
    type MempoolService = DaMempoolService<
        DaPoolAdapter,
        DaPool,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingStorage,
        RuntimeServiceId,
    >;
    type BlobId = BlobId;
    type Tx = SignedMantleTx;

    fn new(mempool_relay: OutboundRelay<<Self::MempoolService as ServiceData>::Message>) -> Self {
        Self {
            mempool_relay,
            _phantom: PhantomData,
        }
    }

    // This adapter implementation uses old da mempool. Until chain service
    // and indexer are updated to use transaction mempool, old da mempool is
    // used for general DA flow testing in integration tests.
    async fn post_tx(
        &self,
        blob_id: Self::BlobId,
        _tx: Self::Tx,
    ) -> Result<(), MempoolAdapterError> {
        // Metadata will not be used in transaction mempool, it's mocked here for da
        // mempool only.
        let metadata = Metadata::new([0; 32], Index::from(0));

        let (reply_channel, receiver) = oneshot::channel();
        self.mempool_relay
            .send(MempoolMsg::Add {
                payload: BlobInfo::new(blob_id, metadata),
                key: blob_id,
                reply_channel,
            })
            .await
            .map_err(|(e, _)| MempoolAdapterError::Other(Box::new(e)))?;

        receiver.await?.map_err(MempoolAdapterError::Mempool)
    }
}
