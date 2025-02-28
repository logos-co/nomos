use core::ops::Range;
use std::{error::Error, fmt::Debug, hash::Hash};

use kzgrs_backend::common::blob::DaBlob;
use nomos_blend_service::network::libp2p::Libp2pAdapter as BlendNetworkAdapter;
use nomos_core::{
    da::{
        blob::{info::DispersedBlobInfo, metadata, select::FillSize as FillSizeWithBlobs, Blob},
        BlobId, DaVerifier as CoreDaVerifier,
    },
    header::HeaderId,
    tx::{select::FillSize as FillSizeWithTx, Transaction},
};
use nomos_da_dispersal::{
    adapters::{mempool::DaMempoolAdapter, network::DispersalNetworkAdapter},
    backend::DispersalBackend,
    DaDispersalMsg, DispersalService,
};
use nomos_da_indexer::{
    consensus::adapters::cryptarchia::CryptarchiaConsensusAdapter,
    storage::adapters::rocksdb::RocksAdapter as IndexerStorageAdapter, DaMsg, DataIndexerService,
};
use nomos_da_network_core::SubnetworkId;
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_da_verifier::{
    backend::VerifierBackend, network::adapters::validator::Libp2pAdapter,
    storage::adapters::rocksdb::RocksAdapter as VerifierStorageAdapter, DaVerifierMsg,
    DaVerifierService,
};
use nomos_libp2p::PeerId;
use nomos_mempool::{
    backend::mockpool::MockPool, network::adapters::libp2p::Libp2pAdapter as MempoolNetworkAdapter,
};
use nomos_storage::backends::{rocksdb::RocksBackend, StorageSerde};
use overwatch_rs::{overwatch::handle::OverwatchHandle, DynError};
use rand::{RngCore, SeedableRng};
use serde::{de::DeserializeOwned, Serialize};
use subnetworks_assignations::MembershipHandler;
use tokio::sync::oneshot;

pub type DaIndexer<
    Tx,
    C,
    V,
    SS,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingRng,
    SamplingStorage,
    const SIZE: usize,
> = DataIndexerService<
    // Indexer specific.
    DaBlob,
    IndexerStorageAdapter<SS, V>,
    CryptarchiaConsensusAdapter<Tx, V>,
    // Cryptarchia specific, should be the same as in `Cryptarchia` type above.
    cryptarchia_consensus::network::adapters::libp2p::LibP2pAdapter<Tx, V>,
    cryptarchia_consensus::blend::adapters::libp2p::LibP2pAdapter<BlendNetworkAdapter, Tx, V>,
    MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
    MempoolNetworkAdapter<Tx, <Tx as Transaction>::Hash>,
    MockPool<HeaderId, V, [u8; 32]>,
    MempoolNetworkAdapter<C, <C as DispersedBlobInfo>::BlobId>,
    FillSizeWithTx<SIZE, Tx>,
    FillSizeWithBlobs<SIZE, V>,
    RocksBackend<SS>,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingRng,
    SamplingStorage,
>;

pub type DaVerifier<Blob, Membership, VerifierBackend, StorageSerializer> = DaVerifierService<
    VerifierBackend,
    Libp2pAdapter<Membership>,
    VerifierStorageAdapter<Blob, StorageSerializer>,
>;

pub type DaDispersal<Backend, NetworkAdapter, MempoolAdapter, Membership, Metadata> =
    DispersalService<Backend, NetworkAdapter, MempoolAdapter, Membership, Metadata>;

pub async fn add_blob<A, B, M, VB, SS>(
    handle: &OverwatchHandle,
    blob: B,
) -> Result<Option<()>, DynError>
where
    A: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    B: Blob + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    <B as Blob>::BlobId: AsRef<[u8]> + Send + Sync + 'static,
    <B as Blob>::ColumnIndex: AsRef<[u8]> + Send + Sync + 'static,
    <B as Blob>::LightBlob: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    <B as Blob>::SharedCommitments: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    M: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    VB: VerifierBackend + CoreDaVerifier<DaBlob = B>,
    <VB as VerifierBackend>::Settings: Clone,
    <VB as CoreDaVerifier>::Error: Error,
    SS: StorageSerde + Send + Sync + 'static,
{
    let relay = handle.relay::<DaVerifier<B, M, VB, SS>>().connect().await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(DaVerifierMsg::AddBlob {
            blob,
            reply_channel: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    Ok(receiver.await?)
}

pub async fn get_range<
    Tx,
    C,
    V,
    SS,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingRng,
    SamplingStorage,
    const SIZE: usize,
>(
    handle: &OverwatchHandle,
    app_id: <V as metadata::Metadata>::AppId,
    range: Range<<V as metadata::Metadata>::Index>,
) -> Result<Vec<(<V as metadata::Metadata>::Index, Vec<DaBlob>)>, DynError>
where
    Tx: Transaction
        + Eq
        + Clone
        + Debug
        + Hash
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <Tx as Transaction>::Hash:
        std::cmp::Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
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
    SamplingRng: SeedableRng + RngCore,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Blob: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter,
{
    let relay = handle
        .relay::<DaIndexer<
            Tx,
            C,
            V,
            SS,
            SamplingBackend,
            SamplingNetworkAdapter,
            SamplingRng,
            SamplingStorage,
            SIZE,
        >>()
        .connect()
        .await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(DaMsg::GetRange {
            app_id,
            range,
            reply_channel: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    Ok(receiver.await?)
}

pub async fn disperse_data<Backend, NetworkAdapter, MempoolAdapter, Membership, Metadata>(
    handle: &OverwatchHandle,
    data: Vec<u8>,
    metadata: Metadata,
) -> Result<(), DynError>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    Backend: DispersalBackend<
            NetworkAdapter = NetworkAdapter,
            MempoolAdapter = MempoolAdapter,
            Metadata = Metadata,
        > + Send
        + Sync,
    Backend::Settings: Clone + Send + Sync,
    NetworkAdapter: DispersalNetworkAdapter<SubnetworkId = Membership::NetworkId> + Send,
    MempoolAdapter: DaMempoolAdapter,
    Metadata: metadata::Metadata + Debug + Send + 'static,
{
    let relay = handle
        .relay::<DaDispersal<Backend, NetworkAdapter, MempoolAdapter, Membership, Metadata>>()
        .connect()
        .await?;
    let (sender, receiver) = oneshot::channel();
    relay
        .send(DaDispersalMsg::Disperse {
            data,
            metadata,
            reply_channel: sender,
        })
        .await
        .map_err(|(e, _)| e)?;

    receiver.await?
}
