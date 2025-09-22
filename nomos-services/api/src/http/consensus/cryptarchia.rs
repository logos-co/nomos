use std::fmt::{Debug, Display};

use chain_service::{
    ConsensusMsg, CryptarchiaConsensus, CryptarchiaInfo,
    network::adapters::libp2p::LibP2pAdapter as ConsensusNetworkAdapter,
};
use kzgrs_backend::dispersal::Metadata;
use nomos_blend_service::ProofsVerifier;
use nomos_core::{
    da::BlobId,
    header::HeaderId,
    mantle::{AuthenticatedMantleTx, Transaction, select::FillSize as FillSizeWithTx},
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_libp2p::PeerId;
use nomos_membership::{
    MembershipService, adapters::sdp::ledger::LedgerSdpAdapter,
    backends::membership::PersistentMembershipBackend,
};
use nomos_mempool::{
    backend::mockpool::MockPool, network::adapters::libp2p::Libp2pAdapter as MempoolNetworkAdapter,
};
use nomos_sdp::backends::mock::MockSdpBackend;
use nomos_storage::backends::rocksdb::RocksBackend;
use overwatch::{overwatch::handle::OverwatchHandle, services::AsServiceId};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::sync::oneshot;

use crate::http::DynError;

pub type Cryptarchia<
    Tx,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    BlendProofsGenerator,
    BlendProofsVerifier,
    RuntimeServiceId,
    const SIZE: usize,
> = CryptarchiaConsensus<
    ConsensusNetworkAdapter<Tx, RuntimeServiceId>,
    BlendService<BlendProofsGenerator, BlendProofsVerifier, RuntimeServiceId>,
    MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
    MempoolNetworkAdapter<Tx, <Tx as Transaction>::Hash, RuntimeServiceId>,
    FillSizeWithTx<SIZE, Tx>,
    RocksBackend,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    RuntimeServiceId,
>;

type BlendService<ProofsGenerator, ProofsVerifier, RuntimeServiceId> = nomos_blend_service::BlendService<
    nomos_blend_service::core::BlendService<
        nomos_blend_service::core::backends::libp2p::Libp2pBlendBackend,
        PeerId,
        nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
        BlendMembershipAdapter<RuntimeServiceId>,
        ProofsGenerator,
        ProofsVerifier,
        RuntimeServiceId,
    >,
    nomos_blend_service::edge::BlendService<
        nomos_blend_service::edge::backends::libp2p::Libp2pBlendBackend,
        PeerId,
        <nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId> as nomos_blend_service::core::network::NetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
        BlendMembershipAdapter<RuntimeServiceId>,
        ProofsGenerator,
        RuntimeServiceId
    >,
    RuntimeServiceId,
>;

pub type DaMembershipStorage<RuntimeServiceId> =
    nomos_membership::adapters::storage::rocksdb::MembershipRocksAdapter<
        RocksBackend,
        RuntimeServiceId,
    >;

type BlendMembershipAdapter<RuntimeServiceId> = nomos_blend_service::membership::service::Adapter<
    MembershipService<
        PersistentMembershipBackend<DaMembershipStorage<RuntimeServiceId>>,
        LedgerSdpAdapter<MockSdpBackend, Metadata, RuntimeServiceId>,
        DaMembershipStorage<RuntimeServiceId>,
        RuntimeServiceId,
    >,
    PeerId,
>;

pub async fn cryptarchia_info<
    Tx,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    BlendProofsGenerator,
    BlendProofsVerifier,
    RuntimeServiceId,
    const SIZE: usize,
>(
    handle: &OverwatchHandle<RuntimeServiceId>,
) -> Result<CryptarchiaInfo, DynError>
where
    Tx: AuthenticatedMantleTx
        + Eq
        + Clone
        + Debug
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <Tx as Transaction>::Hash:
        Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    BlendProofsVerifier: ProofsVerifier + Clone + Send + 'static,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<
            Cryptarchia<
                Tx,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                TimeBackend,
                BlendProofsGenerator,
                BlendProofsVerifier,
                RuntimeServiceId,
                SIZE,
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
    Tx,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingStorage,
    TimeBackend,
    BlendProofsGenerator,
    BlendProofsVerifier,
    RuntimeServiceId,
    const SIZE: usize,
>(
    handle: &OverwatchHandle<RuntimeServiceId>,
    from: Option<HeaderId>,
    to: Option<HeaderId>,
) -> Result<Vec<HeaderId>, DynError>
where
    Tx: AuthenticatedMantleTx
        + Clone
        + Debug
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    <Tx as Transaction>::Hash:
        Ord + Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
    SamplingBackend: DaSamplingServiceBackend<BlobId = BlobId> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    BlendProofsVerifier: ProofsVerifier + Clone + Send + 'static,
    RuntimeServiceId: Debug
        + Send
        + Sync
        + Display
        + 'static
        + AsServiceId<
            Cryptarchia<
                Tx,
                SamplingBackend,
                SamplingNetworkAdapter,
                SamplingStorage,
                TimeBackend,
                BlendProofsGenerator,
                BlendProofsVerifier,
                RuntimeServiceId,
                SIZE,
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
