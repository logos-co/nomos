pub mod consensus;
pub mod storage;

use std::{
    fmt::{Debug, Display, Formatter},
    hash::Hash,
    ops::Range,
};

use nomos_mempool::TransactionId;

use consensus::ConsensusAdapter;
use cryptarchia_consensus::{network::NetworkAdapter, CryptarchiaConsensus};
use futures::StreamExt;
use nomos_core::{
    block::Block,
    da::blob::{info::DispersedBlobInfo, metadata::Metadata, BlobSelect},
    header::HeaderId,
    tx::TxSelect,
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_mempool::{
    backend::{RecoverableMempool},
    network::NetworkAdapter as MempoolAdapter,
    MempoolItem,
};
use nomos_storage::{backends::StorageBackend, StorageService};
use nomos_tracing::info_with_id;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
use rand::{RngCore, SeedableRng};
use serde::{Deserialize, Serialize};
use services_utils::overwatch::lifecycle;
use storage::DaStorageAdapter;
use tokio::sync::oneshot::Sender;
use tracing::instrument;

pub struct DataIndexerService<
    Share,
    DaStorage,
    Consensus,
    NetAdapter,
    BlendAdapter,
    ClPool,
    ClPoolAdapter,
    TxS,
    BS,
    ConsensusStorage,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingRng,
    SamplingStorage,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
    TimeBackend,
    ApiAdapter,
    RuntimeServiceId,
    TxItem,
    DaItem,
    SdpItem,
> where
    Share: 'static,
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Settings: Send,
    BlendAdapter: cryptarchia_consensus::blend::BlendAdapter<RuntimeServiceId>,
    BlendAdapter::Settings: Send,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    ClPool: RecoverableMempool<BlockId = HeaderId, Item = MempoolItem<TxItem, DaItem, SdpItem>>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Item: Clone + Eq + Hash + Debug + 'static,
    ClPool::Key: Debug + 'static,
    ClPool::Settings: Clone,
    NetAdapter::Backend: 'static,
    TxS: TxSelect<Tx = TxItem>,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaItem>,
    BS::Settings: Send,
    DaStorage: DaStorageAdapter<RuntimeServiceId, Info = DaItem, Share = Share>,
    ConsensusStorage: StorageBackend + Send + Sync + 'static,
    SamplingRng: SeedableRng + RngCore,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = ClPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
    TxItem: 'static,
    DaItem: Metadata + Debug + 'static,
    SdpItem: 'static,
{
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
}

pub enum DaMsg<Blob, Meta: Metadata> {
    AddIndex {
        info: Meta,
    },
    GetRange {
        app_id: <Meta as Metadata>::AppId,
        range: Range<<Meta as Metadata>::Index>,
        reply_channel: Sender<Vec<(<Meta as Metadata>::Index, Vec<Blob>)>>,
    },
}

impl<Blob: 'static, Meta: Metadata + 'static> Debug for DaMsg<Blob, Meta> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AddIndex { .. } => {
                write!(f, "DaMsg::AddIndex")
            }
            Self::GetRange { .. } => {
                write!(f, "DaMsg::GetRange")
            }
        }
    }
}

impl<
        Share,
        DaStorage,
        Consensus,
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        TxS,
        BS,
        ConsensusStorage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
        TxItem,
        DaItem,
        SdpItem,
    > ServiceData
    for DataIndexerService<
        Share,
        DaStorage,
        Consensus,
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        TxS,
        BS,
        ConsensusStorage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
        TxItem,
        DaItem,
        SdpItem,
    >
where
    Share: 'static,
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Settings: Send,
    BlendAdapter: cryptarchia_consensus::blend::BlendAdapter<RuntimeServiceId>,
    BlendAdapter::Settings: Send,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    ClPool: RecoverableMempool<BlockId = HeaderId, Item = MempoolItem<TxItem, DaItem, SdpItem>>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Item: Clone + Eq + Hash + Debug + 'static,
    ClPool::Key: Debug + 'static,
    ClPool::Settings: Clone,
    NetAdapter::Backend: 'static,
    TxS: TxSelect<Tx = TxItem>,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaItem>,
    BS::Settings: Send,
    DaStorage: DaStorageAdapter<RuntimeServiceId, Info = DaItem, Share = Share>,
    ConsensusStorage: StorageBackend + Send + Sync + 'static,
    SamplingRng: SeedableRng + RngCore,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = ClPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
    DaItem: Metadata + Debug,
{
    type Settings = IndexerSettings<DaStorage::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaMsg<Share, DaItem>;
}

impl<
        Share,
        DaStorage,
        Consensus,
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        TxS,
        BS,
        ConsensusStorage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
        TxItem,
        DaItem,
        SdpItem,
    >
    DataIndexerService<
        Share,
        DaStorage,
        Consensus,
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        TxS,
        BS,
        ConsensusStorage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
        TxItem,
        DaItem,
        SdpItem,
    >
where
    Share: Send + Sync + 'static,
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Settings: Send,
    BlendAdapter: cryptarchia_consensus::blend::BlendAdapter<RuntimeServiceId>,
    BlendAdapter::Settings: Send,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    ClPool: RecoverableMempool<BlockId = HeaderId, Item = MempoolItem<TxItem, DaItem, SdpItem>>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Item: Clone + Eq + Hash + Debug + 'static,
    ClPool::Key: Debug + 'static,
    ClPool::Settings: Clone,
    NetAdapter::Backend: 'static,
    TxS: TxSelect<Tx = TxItem>,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaItem>,
    BS::Settings: Send,
    DaStorage: DaStorageAdapter<RuntimeServiceId, Info = DaItem, Share = Share> + Sync,
    ConsensusStorage: StorageBackend + Send + Sync + 'static,
    SamplingRng: SeedableRng + RngCore,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = ClPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId>,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
    TxItem: Eq + Clone + Hash,
    DaItem: DispersedBlobInfo + Metadata + Clone + Eq + Hash + Debug + 'static,
    <DaItem as DispersedBlobInfo>::BlobId: AsRef<[u8]>,
    SdpItem: Eq + Clone + Hash,
{
    #[instrument(skip_all)]
    async fn handle_new_block(
        storage_adapter: &DaStorage,
        block: Block<TxItem, DaItem>,
    ) -> Result<(), DynError> {
        for info in block.blobs() {
            info_with_id!(info.blob_id().as_ref(), "HandleNewBlock");
            storage_adapter.add_index(info).await?;
        }
        Ok(())
    }

    #[instrument(skip_all)]
    async fn handle_da_msg(
        storage_adapter: &DaStorage,
        msg: DaMsg<Share, DaItem>,
    ) -> Result<(), DynError> {
        match msg {
            DaMsg::AddIndex { info } => {
                info_with_id!(info.blob_id().as_ref(), "AddIndex");
                storage_adapter.add_index(&info).await
            }
            DaMsg::GetRange {
                app_id,
                range,
                reply_channel,
            } => {
                let stream = storage_adapter.get_range_stream(app_id, range).await;
                let results = stream.collect::<Vec<_>>().await;

                reply_channel
                    .send(results)
                    .map_err(|_| "Error sending range response".into())
            }
        }
    }
}

#[async_trait::async_trait]
impl<
        Share,
        DaStorage,
        Consensus,
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        TxS,
        BS,
        ConsensusStorage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
        TxItem,
        DaItem,
        SdpItem,
    > ServiceCore<RuntimeServiceId>
    for DataIndexerService<
        Share,
        DaStorage,
        Consensus,
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        TxS,
        BS,
        ConsensusStorage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
        TxItem,
        DaItem,
        SdpItem,
    >
where
    Share: Debug + Send + Sync,
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::Settings: Send,
    BlendAdapter: cryptarchia_consensus::blend::BlendAdapter<RuntimeServiceId>,
    BlendAdapter::Settings: Send,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    ClPool: RecoverableMempool<BlockId = HeaderId, Item = MempoolItem<TxItem, DaItem, SdpItem>>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Key: Debug + 'static,
    ClPool::Settings: Clone,
    ClPool::Item:
        Debug + Clone + Eq + Hash + Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    NetAdapter::Backend: 'static,
    TxS: TxSelect<Tx = TxItem>,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaItem>,
    BS::Settings: Send,
    DaStorage:
        DaStorageAdapter<RuntimeServiceId, Info = DaItem, Share = Share> + Send + Sync + 'static,
    DaStorage::Settings: Clone + Send + Sync + 'static,
    ConsensusStorage: StorageBackend + Send + Sync + 'static,
    Consensus: ConsensusAdapter<Tx = TxItem, Cert = DaItem> + Send + Sync,
    SamplingRng: SeedableRng + RngCore,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = ClPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId>,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + Send
        + AsServiceId<Self>
        + AsServiceId<
            CryptarchiaConsensus<
                NetAdapter,
                BlendAdapter,
                ClPool,
                ClPoolAdapter,
                TxS,
                BS,
                ConsensusStorage,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingRng,
                SamplingStorage,
                DaVerifierBackend,
                DaVerifierNetwork,
                DaVerifierStorage,
                TimeBackend,
                ApiAdapter,
                RuntimeServiceId,
                TxItem,
                DaItem,
                SdpItem,
            >,
        > + AsServiceId<StorageService<DaStorage::Backend, RuntimeServiceId>>,
    TxItem: TransactionId<ClPool::Key> + Eq + Clone + Hash + Send,
    DaItem: TransactionId<ClPool::Key>
        + DispersedBlobInfo
        + Metadata
        + Clone
        + Eq
        + Hash
        + Debug
        + Send
        + Sync
        + 'static,
    <DaItem as DispersedBlobInfo>::BlobId: AsRef<[u8]>,
    <DaItem as Metadata>::Index: Send,
    <DaItem as Metadata>::AppId: Send,
    SdpItem: TransactionId<ClPool::Key> + Eq + Clone + Hash,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self { service_state })
    }

    async fn run(self) -> Result<(), DynError> {
        let Self { mut service_state } = self;

        let consensus_relay = service_state
            .overwatch_handle
            .relay::<CryptarchiaConsensus<_, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _, _>>(
            )
            .await
            .expect("Relay connection with ConsensusService should succeed");
        let storage_relay = service_state
            .overwatch_handle
            .relay::<StorageService<_, _>>()
            .await
            .expect("Relay connection with StorageService should succeed");

        let consensus_adapter = Consensus::new(consensus_relay).await;
        let mut consensus_blocks = consensus_adapter.block_stream().await;
        let storage_adapter = DaStorage::new(storage_relay).await;

        let mut lifecycle_stream = service_state.lifecycle_handle.message_stream();
        loop {
            tokio::select! {
                Some(block) = consensus_blocks.next() => {
                    if let Err(e) = Self::handle_new_block(&storage_adapter, block).await {
                        tracing::debug!("Failed to add  a new received block: {e:?}");
                    }
                }
                Some(msg) = service_state.inbound_relay.recv() => {
                    if let Err(e) = Self::handle_da_msg(&storage_adapter, msg).await {
                        tracing::debug!("Failed to handle da msg: {e:?}");
                    }
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerSettings<S> {
    pub storage: S,
}
