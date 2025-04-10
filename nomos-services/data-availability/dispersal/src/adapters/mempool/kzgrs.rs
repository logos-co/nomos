use std::{fmt::Debug, hash::Hash, marker::PhantomData};

use kzgrs_backend::dispersal::{self, BlobInfo};
use nomos_core::{
    da::{blob::info::DispersedBlobInfo, BlobId},
    header::HeaderId,
};
use nomos_mempool::{
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as MempoolAdapter,
    service::MempoolService,
    MempoolItem, MempoolMsg,
};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use super::{DaMempoolAdapter, DaMempoolAdapterError};

type MempoolRelay<Key, TxItem, DaItem, SdpItem> =
    OutboundRelay<MempoolMsg<HeaderId, Key, TxItem, DaItem, SdpItem>>;

pub struct KzgrsMempoolAdapter<DaPoolAdapter, DaPool, RuntimeServiceId, TxItem, DaItem, SdpItem>
where
    DaPool: MemPool<BlockId = HeaderId>,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    DaPool::Item: Clone + Eq + Hash + Debug + 'static,
    DaPool::Key: Debug + 'static,
{
    pub mempool_relay: MempoolRelay<DaPool::Key, TxItem, DaItem, SdpItem>,
    _phantom: PhantomData<(DaPoolAdapter, RuntimeServiceId)>,
}

#[async_trait::async_trait]
impl<DaPoolAdapter, DaPool, RuntimeServiceId, TxItem, DaItem, SdpItem> DaMempoolAdapter
    for KzgrsMempoolAdapter<DaPoolAdapter, DaPool, RuntimeServiceId, TxItem, BlobInfo, SdpItem>
where
    DaPool: RecoverableMempool<BlockId = HeaderId, Key = BlobId>,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPoolAdapter: MempoolAdapter<
            RuntimeServiceId,
            Key = DaPool::Key,
            Payload = MempoolItem<TxItem, DaItem, SdpItem>,
        > + Sync,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug + Send,
    DaPool::Item: Clone + Eq + Hash + Debug + Send + 'static,
    DaPool::Key: Debug + Send + 'static,
    DaPool::Settings: Clone,
    RuntimeServiceId: Sync,
    TxItem: Send,
    DaItem: Send,
    SdpItem: Send,
{
    type MempoolService =
        MempoolService<DaPoolAdapter, DaPool, RuntimeServiceId, TxItem, BlobInfo, SdpItem>;
    type BlobId = BlobId;
    type Metadata = dispersal::Metadata;

    fn new(mempool_relay: OutboundRelay<<Self::MempoolService as ServiceData>::Message>) -> Self {
        Self {
            mempool_relay,
            _phantom: PhantomData,
        }
    }

    async fn post_blob_id(
        &self,
        blob_id: Self::BlobId,
        metadata: Self::Metadata,
    ) -> Result<(), DaMempoolAdapterError> {
        let (reply_channel, receiver) = oneshot::channel();
        self.mempool_relay
            .send(MempoolMsg::Add {
                item: MempoolItem::Da(BlobInfo::new(blob_id, metadata)),
                key: blob_id,
                reply_channel,
            })
            .await
            .map_err(|(e, _)| DaMempoolAdapterError::from(e))?;

        receiver.await?.map_err(DaMempoolAdapterError::Mempool)
    }
}
