use std::{fmt::Debug, marker::PhantomData};

use kzgrs_backend::dispersal::{self, BlobInfo};
use nomos_core::{
    da::{blob::info::DispersedBlobInfo, BlobId},
    header::HeaderId,
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_mempool::{
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as MempoolAdapter,
    DaMempoolService, MempoolMsg,
};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use rand::{RngCore, SeedableRng};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use super::{DaMempoolAdapter, DaMempoolAdapterError};

type MempoolRelay<Payload, Item, Key> = OutboundRelay<MempoolMsg<HeaderId, Payload, Item, Key>>;

pub struct KzgrsMempoolAdapter<
    DaPoolAdapter,
    DaPool,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingRng,
    SamplingStorage,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
    ApiAdapter,
    RuntimeServiceId,
> where
    DaPool: MemPool<BlockId = HeaderId>,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    DaPool::Item: Clone + Eq + Debug + 'static,
    DaPool::Key: Debug + 'static,
{
    pub mempool_relay: MempoolRelay<DaPoolAdapter::Payload, DaPool::Item, DaPool::Key>,
    _phantom: PhantomData<(
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
    )>,
    _phantom2: PhantomData<(DaVerifierBackend, DaVerifierNetwork, DaVerifierStorage)>,
    _phantom3: PhantomData<ApiAdapter>,
}

#[async_trait::async_trait]
impl<
        DaPoolAdapter,
        DaPool,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        ApiAdapter,
        RuntimeServiceId,
    > DaMempoolAdapter
    for KzgrsMempoolAdapter<
        DaPoolAdapter,
        DaPool,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        ApiAdapter,
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
    SamplingRng: SeedableRng + RngCore + Sync,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = DaPool::Key> + Send + Sync,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter:
        nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierNetwork::Settings: Clone,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
{
    type MempoolService = DaMempoolService<
        DaPoolAdapter,
        DaPool,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        ApiAdapter,
        RuntimeServiceId,
    >;
    type BlobId = BlobId;
    type Metadata = dispersal::Metadata;

    fn new(mempool_relay: OutboundRelay<<Self::MempoolService as ServiceData>::Message>) -> Self {
        Self {
            mempool_relay,
            _phantom: PhantomData,
            _phantom2: PhantomData,
            _phantom3: PhantomData,
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
                payload: BlobInfo::new(blob_id, metadata),
                key: blob_id,
                reply_channel,
            })
            .await
            .map_err(|(e, _)| DaMempoolAdapterError::from(e))?;

        receiver.await?.map_err(DaMempoolAdapterError::Mempool)
    }
}
