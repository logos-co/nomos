use core::ops::Range;
use std::{
    error::Error,
    fmt::{Debug, Display},
    hash::Hash,
};

use kzgrs_backend::common::share::DaShare;
use nomos_core::{
    block::BlockNumber,
    da::{
        blob::{info::DispersedBlobInfo, metadata, select::FillSize as FillSizeWithBlobs, Share},
        BlobId, DaVerifier as CoreDaVerifier,
    },
    header::HeaderId,
    mantle::{select::FillSize as FillSizeWithTx, SignedMantleTx, Transaction},
};
use nomos_da_dispersal::{
    adapters::network::DispersalNetworkAdapter, backend::DispersalBackend, DaDispersalMsg,
    DispersalService,
};
use nomos_da_indexer::{
    consensus::adapters::cryptarchia::CryptarchiaConsensusAdapter,
    storage::adapters::rocksdb::RocksAdapter as IndexerStorageAdapter, DaMsg, DataIndexerService,
};
use nomos_da_network_core::{maintenance::monitor::ConnectionMonitorCommand, SubnetworkId};
use nomos_da_network_service::{
    api::ApiAdapter as ApiAdapterTrait,
    backends::{
        libp2p::{executor::ExecutorDaNetworkMessage, validator::DaNetworkMessage},
        NetworkBackend,
    },
    DaNetworkMsg, MembershipResponse, NetworkService,
};
use nomos_da_sampling::{
    backend::DaSamplingServiceBackend, storage::adapters::rocksdb::converter::DaStorageConverter,
};
use nomos_da_verifier::{
    backend::VerifierBackend, mempool::DaMempoolAdapter,
    storage::adapters::rocksdb::RocksAdapter as VerifierStorageAdapter, DaVerifierMsg,
    DaVerifierService,
};
use nomos_libp2p::PeerId;
use nomos_mempool::{
    backend::mockpool::MockPool, network::adapters::libp2p::Libp2pAdapter as MempoolNetworkAdapter,
};
use nomos_storage::{
    api::da::DaConverter,
    backends::{rocksdb::RocksBackend, StorageSerde},
};
use overwatch::{overwatch::handle::OverwatchHandle, services::AsServiceId, DynError};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use subnetworks_assignations::MembershipHandler;
use tokio::sync::oneshot;

use crate::wait_with_timeout;

pub type DaIndexer<
    Tx,
    C,
    V,
    SS,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
    const SIZE: usize,
> = DataIndexerService<
    // Indexer specific.
    DaShare,
    IndexerStorageAdapter<SS, V, DaStorageConverter>,
    CryptarchiaConsensusAdapter<Tx, V>,
    // Cryptarchia specific, should be the same as in `Cryptarchia` type above.
    chain_service::network::adapters::libp2p::LibP2pAdapter<Tx, V, RuntimeServiceId>,
    BlendService<RuntimeServiceId>,
    MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
    MempoolNetworkAdapter<Tx, <Tx as Transaction>::Hash, RuntimeServiceId>,
    MockPool<HeaderId, V, [u8; 32]>,
    MempoolNetworkAdapter<C, <C as DispersedBlobInfo>::BlobId, RuntimeServiceId>,
    FillSizeWithTx<SIZE, Tx>,
    FillSizeWithBlobs<SIZE, V>,
    RocksBackend<SS>,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
>;

type BlendService<RuntimeServiceId> = nomos_blend_service::BlendService<
    nomos_blend_service::core::BlendService<
        nomos_blend_service::core::backends::libp2p::Libp2pBlendBackend,
        PeerId,
        nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
        RuntimeServiceId,
    >,
    nomos_blend_service::edge::BlendService<
        nomos_blend_service::edge::backends::libp2p::Libp2pBlendBackend,
        PeerId,
        <nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId> as nomos_blend_service::core::network::NetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
        RuntimeServiceId
    >,
    RuntimeServiceId,
>;

pub type DaVerifier<
    Blob,
    NetworkAdapter,
    VerifierBackend,
    StorageSerializer,
    DaStorageConverter,
    VerifierMempoolAdapter,
    RuntimeServiceId,
> = DaVerifierService<
    VerifierBackend,
    NetworkAdapter,
    VerifierStorageAdapter<Blob, StorageSerializer, DaStorageConverter>,
    VerifierMempoolAdapter,
    RuntimeServiceId,
>;

pub type DaDispersal<Backend, NetworkAdapter, Membership, RuntimeServiceId> =
    DispersalService<Backend, NetworkAdapter, Membership, RuntimeServiceId>;

pub type DaNetwork<
    Backend,
    Membership,
    MembershipAdapter,
    StorageAdapter,
    ApiAdapter,
    RuntimeServiceId,
> = NetworkService<
    Backend,
    Membership,
    MembershipAdapter,
    StorageAdapter,
    ApiAdapter,
    RuntimeServiceId,
>;

pub async fn add_share<
    DaShare,
    VerifierNetwork,
    ShareVerifier,
    SerdeOp,
    DaStorageConverter,
    VerifierMempoolAdapter,
    RuntimeServiceId,
>(
    handle: &OverwatchHandle<RuntimeServiceId>,
    share: DaShare,
) -> Result<Option<()>, DynError>
where
    DaShare: Share + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    <DaShare as Share>::BlobId: Clone + Send + Sync + 'static,
    <DaShare as Share>::ShareIndex: Clone + Eq + Hash + Send + Sync + 'static,
    <DaShare as Share>::LightShare: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    <DaShare as Share>::SharesCommitments:
        Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    VerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId>,
    VerifierNetwork::Settings: Clone,
    ShareVerifier: VerifierBackend + CoreDaVerifier<DaShare = DaShare>,
    <ShareVerifier as VerifierBackend>::Settings: Clone,
    <ShareVerifier as CoreDaVerifier>::Error: Error,
    SerdeOp: StorageSerde + Send + Sync + 'static,
    DaStorageConverter: DaConverter<RocksBackend<SerdeOp>, Share = DaShare, Tx = SignedMantleTx>
        + Send
        + Sync
        + 'static,
    VerifierMempoolAdapter: DaMempoolAdapter,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + AsServiceId<
            DaVerifier<
                DaShare,
                VerifierNetwork,
                ShareVerifier,
                SerdeOp,
                DaStorageConverter,
                VerifierMempoolAdapter,
                RuntimeServiceId,
            >,
        >,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(DaVerifierMsg::AddShare {
            share,
            reply_channel: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    wait_with_timeout(receiver, "Timeout while waiting for add share".to_owned()).await
}

pub async fn get_range<
    Tx,
    C,
    V,
    SS,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
    const SIZE: usize,
>(
    handle: &OverwatchHandle<RuntimeServiceId>,
    app_id: <V as metadata::Metadata>::AppId,
    range: Range<<V as metadata::Metadata>::Index>,
) -> Result<Vec<(<V as metadata::Metadata>::Index, Vec<DaShare>)>, DynError>
where
    Tx: Transaction + Eq + Clone + Debug + Serialize + DeserializeOwned + Send + Sync + 'static,
    <Tx as Transaction>::Hash:
        Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    C: DispersedBlobInfo<BlobId = [u8; 32]>
        + Clone
        + Debug
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <C as DispersedBlobInfo>::BlobId: Clone + Send + Sync,
    V: DispersedBlobInfo<BlobId = [u8; 32]>
        + From<C>
        + Eq
        + Debug
        + metadata::Metadata
        + Hash
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <V as DispersedBlobInfo>::BlobId: Debug + Clone + Ord + Hash,
    <V as metadata::Metadata>::AppId: AsRef<[u8]> + Serialize + Clone + Send + Sync,
    <V as metadata::Metadata>::Index:
        AsRef<[u8]> + Serialize + DeserializeOwned + Clone + PartialOrd + Send + Sync,
    SS: StorageSerde + Send + Sync + 'static,
    <SS as StorageSerde>::Error: Error + Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<BlobId = BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + 'static
        + AsServiceId<
            DaIndexer<
                Tx,
                C,
                V,
                SS,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                TimeBackend,
                RuntimeServiceId,
                SIZE,
            >,
        >,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(DaMsg::GetRange {
            app_id,
            range,
            reply_channel: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    wait_with_timeout(receiver, "Timeout while waiting for get range".to_owned()).await
}

pub async fn disperse_data<Backend, NetworkAdapter, Membership, RuntimeServiceId>(
    handle: &OverwatchHandle<RuntimeServiceId>,
    data: Vec<u8>,
) -> Result<Backend::BlobId, DynError>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    Backend: DispersalBackend<NetworkAdapter = NetworkAdapter> + Send + Sync + 'static,
    Backend::Settings: Clone + Send + Sync,
    Backend::BlobId: Serialize,
    NetworkAdapter: DispersalNetworkAdapter<SubnetworkId = Membership::NetworkId> + Send,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + AsServiceId<DaDispersal<Backend, NetworkAdapter, Membership, RuntimeServiceId>>,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(DaDispersalMsg::Disperse {
            data,
            reply_channel: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    wait_with_timeout(
        receiver,
        "Timeout while waiting for disperse data".to_owned(),
    )
    .await?
}

pub async fn block_peer<
    Backend,
    Membership,
    MembershipAdapter,
    StorageAdapter,
    ApiAdapter,
    RuntimeServiceId,
>(
    handle: &OverwatchHandle<RuntimeServiceId>,
    peer_id: PeerId,
) -> Result<bool, DynError>
where
    Backend: NetworkBackend<RuntimeServiceId> + 'static + Send,
    Backend::Message: MonitorMessageFactory,
    Membership: MembershipHandler + Clone + Send + Sync + 'static,
    Membership::Id: Send + Sync + 'static,
    Membership::NetworkId: Send + Sync + 'static,
    ApiAdapter: ApiAdapterTrait + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + 'static
        + AsServiceId<
            NetworkService<
                Backend,
                Membership,
                MembershipAdapter,
                StorageAdapter,
                ApiAdapter,
                RuntimeServiceId,
            >,
        >,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    let message = Backend::Message::create_block_message(peer_id, sender);
    relay
        .send(DaNetworkMsg::Process(message))
        .await
        .map_err(|(e, _)| e)?;

    wait_with_timeout(receiver, "Timeout while waiting for block peer".to_owned()).await
}

pub async fn unblock_peer<
    Backend,
    Membership,
    MembershipAdapter,
    StorageAdapter,
    ApiAdapter,
    RuntimeServiceId,
>(
    handle: &OverwatchHandle<RuntimeServiceId>,
    peer_id: PeerId,
) -> Result<bool, DynError>
where
    Backend: NetworkBackend<RuntimeServiceId> + 'static + Send,
    Backend::Message: MonitorMessageFactory,
    Membership: MembershipHandler + Clone + Send + Sync + 'static,
    Membership::Id: Send + Sync + 'static,
    Membership::NetworkId: Send + Sync + 'static,
    ApiAdapter: ApiAdapterTrait + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + 'static
        + AsServiceId<
            NetworkService<
                Backend,
                Membership,
                MembershipAdapter,
                StorageAdapter,
                ApiAdapter,
                RuntimeServiceId,
            >,
        >,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    let message = Backend::Message::create_unblock_message(peer_id, sender);
    relay
        .send(DaNetworkMsg::Process(message))
        .await
        .map_err(|(e, _)| e)?;

    wait_with_timeout(
        receiver,
        "Timeout while waiting for unblock peer".to_owned(),
    )
    .await
}

pub async fn blacklisted_peers<
    Backend,
    Membership,
    MembershipAdapter,
    StorageAdapter,
    ApiAdapter,
    RuntimeServiceId,
>(
    handle: &OverwatchHandle<RuntimeServiceId>,
) -> Result<Vec<PeerId>, DynError>
where
    Backend: NetworkBackend<RuntimeServiceId> + 'static + Send,
    Backend::Message: MonitorMessageFactory,
    Membership: MembershipHandler + Clone + Send + Sync + 'static,
    Membership::Id: Send + Sync + 'static,
    Membership::NetworkId: Send + Sync + 'static,
    ApiAdapter: ApiAdapterTrait + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + 'static
        + AsServiceId<
            NetworkService<
                Backend,
                Membership,
                MembershipAdapter,
                StorageAdapter,
                ApiAdapter,
                RuntimeServiceId,
            >,
        >,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    let message = Backend::Message::create_blacklisted_message(sender);
    relay
        .send(DaNetworkMsg::Process(message))
        .await
        .map_err(|(e, _)| e)?;

    wait_with_timeout(
        receiver,
        "Timeout while waiting for blacklisted peers".to_owned(),
    )
    .await
}

pub async fn da_get_membership<
    Backend,
    Membership,
    MembershipAdapter,
    StorageAdapter,
    ApiAdapter,
    RuntimeServiceId,
>(
    handle: OverwatchHandle<RuntimeServiceId>,
    block_number: BlockNumber,
) -> Result<MembershipResponse, DynError>
where
    Backend: NetworkBackend<RuntimeServiceId> + 'static + Send,
    Membership: MembershipHandler + Clone + Send + Sync + 'static,
    Membership::Id: Send + Sync + 'static,
    Membership::NetworkId: Send + Sync + 'static,
    ApiAdapter: ApiAdapterTrait + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + 'static
        + AsServiceId<
            NetworkService<
                Backend,
                Membership,
                MembershipAdapter,
                StorageAdapter,
                ApiAdapter,
                RuntimeServiceId,
            >,
        >,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    let message = DaNetworkMsg::GetMembership {
        block_number,
        sender,
    };
    relay.send(message).await.map_err(|(e, _)| e)?;

    wait_with_timeout(
        receiver,
        "Timeout while waiting for get membership".to_owned(),
    )
    .await
}

pub async fn balancer_stats<
    Backend,
    Membership,
    MembershipAdapter,
    StorageAdapter,
    ApiAdapter,
    RuntimeServiceId,
>(
    handle: &OverwatchHandle<RuntimeServiceId>,
) -> Result<<Backend::Message as BalancerMessageFactory>::BalancerStats, DynError>
where
    Backend: NetworkBackend<RuntimeServiceId> + 'static + Send,
    Backend::Message: BalancerMessageFactory,
    Membership: MembershipHandler + Clone + Send + Sync + 'static,
    Membership::Id: Send + Sync + 'static,
    Membership::NetworkId: Send + Sync + 'static,
    ApiAdapter: ApiAdapterTrait + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + 'static
        + AsServiceId<
            NetworkService<
                Backend,
                Membership,
                MembershipAdapter,
                StorageAdapter,
                ApiAdapter,
                RuntimeServiceId,
            >,
        >,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    let message = Backend::Message::create_stats_message(sender);
    relay
        .send(DaNetworkMsg::Process(message))
        .await
        .map_err(|(e, _)| e)?;

    wait_with_timeout(
        receiver,
        "Timeout while waiting for balancer stats".to_owned(),
    )
    .await
}

pub async fn monitor_stats<
    Backend,
    Membership,
    MembershipAdapter,
    StorageAdapter,
    ApiAdapter,
    RuntimeServiceId,
>(
    handle: &OverwatchHandle<RuntimeServiceId>,
) -> Result<<Backend::Message as MonitorMessageFactory>::MonitorStats, DynError>
where
    Backend: NetworkBackend<RuntimeServiceId> + 'static + Send,
    Backend::Message: MonitorMessageFactory,
    Membership: MembershipHandler + Clone + Send + Sync + 'static,
    Membership::Id: Send + Sync + 'static,
    Membership::NetworkId: Send + Sync + 'static,
    ApiAdapter: ApiAdapterTrait + Send + Sync + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + 'static
        + AsServiceId<
            NetworkService<
                Backend,
                Membership,
                MembershipAdapter,
                StorageAdapter,
                ApiAdapter,
                RuntimeServiceId,
            >,
        >,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();
    let message = Backend::Message::create_stats_message(sender);
    relay
        .send(DaNetworkMsg::Process(message))
        .await
        .map_err(|(e, _)| e)?;

    wait_with_timeout(
        receiver,
        "Timeout while waiting for monitor stats".to_owned(),
    )
    .await
}

// Factory for generating messages for connection monitor (validator and
// executor).
pub trait MonitorMessageFactory {
    type MonitorStats: Debug + Serialize;

    fn create_block_message(peer_id: PeerId, sender: oneshot::Sender<bool>) -> Self;
    fn create_unblock_message(peer_id: PeerId, sender: oneshot::Sender<bool>) -> Self;
    fn create_blacklisted_message(sender: oneshot::Sender<Vec<PeerId>>) -> Self;
    fn create_stats_message(sender: oneshot::Sender<Self::MonitorStats>) -> Self;
}

impl<BalancerStats, MonitorStats> MonitorMessageFactory
    for DaNetworkMessage<BalancerStats, MonitorStats>
where
    BalancerStats: Debug + Serialize,
    MonitorStats: Debug + Serialize,
{
    type MonitorStats = MonitorStats;

    fn create_block_message(peer_id: PeerId, sender: oneshot::Sender<bool>) -> Self {
        Self::MonitorRequest(ConnectionMonitorCommand::Block(peer_id, sender))
    }

    fn create_unblock_message(peer_id: PeerId, sender: oneshot::Sender<bool>) -> Self {
        Self::MonitorRequest(ConnectionMonitorCommand::Unblock(peer_id, sender))
    }

    fn create_blacklisted_message(sender: oneshot::Sender<Vec<PeerId>>) -> Self {
        Self::MonitorRequest(ConnectionMonitorCommand::BlacklistedPeers(sender))
    }

    fn create_stats_message(sender: oneshot::Sender<Self::MonitorStats>) -> Self {
        Self::MonitorRequest(ConnectionMonitorCommand::Stats(sender))
    }
}

impl<BalancerStats, MonitorStats> MonitorMessageFactory
    for ExecutorDaNetworkMessage<BalancerStats, MonitorStats>
where
    BalancerStats: Debug + Serialize,
    MonitorStats: Debug + Serialize,
{
    type MonitorStats = MonitorStats;

    fn create_block_message(peer_id: PeerId, sender: oneshot::Sender<bool>) -> Self {
        Self::MonitorRequest(ConnectionMonitorCommand::Block(peer_id, sender))
    }

    fn create_unblock_message(peer_id: PeerId, sender: oneshot::Sender<bool>) -> Self {
        Self::MonitorRequest(ConnectionMonitorCommand::Unblock(peer_id, sender))
    }

    fn create_blacklisted_message(sender: oneshot::Sender<Vec<PeerId>>) -> Self {
        Self::MonitorRequest(ConnectionMonitorCommand::BlacklistedPeers(sender))
    }

    fn create_stats_message(sender: oneshot::Sender<Self::MonitorStats>) -> Self {
        Self::MonitorRequest(ConnectionMonitorCommand::Stats(sender))
    }
}

pub trait BalancerMessageFactory {
    type BalancerStats: Debug + Serialize;

    fn create_stats_message(sender: oneshot::Sender<Self::BalancerStats>) -> Self;
}

impl<BalancerStats, MonitorStats> BalancerMessageFactory
    for DaNetworkMessage<BalancerStats, MonitorStats>
where
    BalancerStats: Debug + Serialize,
{
    type BalancerStats = BalancerStats;

    fn create_stats_message(sender: oneshot::Sender<BalancerStats>) -> Self {
        Self::BalancerStats(sender)
    }
}

impl<BalancerStats, MonitorStats> BalancerMessageFactory
    for ExecutorDaNetworkMessage<BalancerStats, MonitorStats>
where
    BalancerStats: Debug + Serialize,
{
    type BalancerStats = BalancerStats;

    fn create_stats_message(sender: oneshot::Sender<BalancerStats>) -> Self {
        Self::BalancerStats(sender)
    }
}
