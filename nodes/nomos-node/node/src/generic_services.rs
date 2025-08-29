use chain_service::CryptarchiaConsensus;
use kzgrs_backend::{
    common::share::DaShare,
    dispersal::{BlobInfo, Metadata},
};
use nomos_core::{
    da::blob::info::DispersedBlobInfo,
    header::HeaderId,
    mantle::{SignedMantleTx, Transaction},
};
use nomos_da_network_service::{
    membership::adapters::service::MembershipServiceAdapter,
    storage::adapters::rocksdb::RocksAdapter,
};
use nomos_da_sampling::{
    backend::kzgrs::KzgrsSamplingBackend, storage::adapters::rocksdb::converter::DaStorageConverter,
};
use nomos_da_verifier::{backend::kzgrs::KzgrsDaVerifier, mempool::kzgrs::KzgrsMempoolAdapter};
use nomos_libp2p::PeerId;
use nomos_membership::{adapters::sdp::LedgerSdpAdapter, backends::mock::MockMembershipBackend};
use nomos_mempool::backend::mockpool::MockPool;
use nomos_sdp::backends::mock::MockSdpBackend;
use nomos_storage::backends::rocksdb::RocksBackend;
use nomos_time::backends::NtpTimeBackend;

use crate::{Wire, MB16};

pub type TxMempoolService<SamplingNetworkAdapter, RuntimeServiceId> =
    nomos_mempool::TxMempoolService<
        nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
            SignedMantleTx,
            <SignedMantleTx as Transaction>::Hash,
            RuntimeServiceId,
        >,
        SamplingNetworkAdapter,
        nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        MockPool<HeaderId, SignedMantleTx, <SignedMantleTx as Transaction>::Hash>,
        RuntimeServiceId,
    >;

pub type TimeService<RuntimeServiceId> = nomos_time::TimeService<NtpTimeBackend, RuntimeServiceId>;

pub type BlendService<RuntimeServiceId> = nomos_blend_service::BlendService<
    nomos_blend_service::core::BlendService<
        nomos_blend_service::core::backends::libp2p::Libp2pBlendBackend,
        PeerId,
        nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
        BlendMembershipAdapter<RuntimeServiceId>,
        RuntimeServiceId,
    >,
    nomos_blend_service::edge::BlendService<
        nomos_blend_service::edge::backends::libp2p::Libp2pBlendBackend,
        PeerId,
        <nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId> as nomos_blend_service::core::network::NetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
        BlendMembershipAdapter<RuntimeServiceId>,
        RuntimeServiceId
    >,
    RuntimeServiceId,
>;

type BlendMembershipAdapter<RuntimeServiceId> =
    nomos_blend_service::membership::service::Adapter<MembershipService<RuntimeServiceId>, PeerId>;

pub type VerifierMempoolAdapter<NetworkAdapter, RuntimeServiceId> = KzgrsMempoolAdapter<
    nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
        BlobInfo,
        <BlobInfo as DispersedBlobInfo>::BlobId,
        RuntimeServiceId,
    >,
    MockPool<HeaderId, BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
    KzgrsSamplingBackend,
    NetworkAdapter,
    nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire, DaStorageConverter>,
    RuntimeServiceId,
>;

pub type DaVerifierService<VerifierAdapter, MempoolAdapter, RuntimeServiceId> =
    nomos_da_verifier::DaVerifierService<
        KzgrsDaVerifier,
        VerifierAdapter,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        MempoolAdapter,
        RuntimeServiceId,
    >;

pub type DaSamplingService<SamplingAdapter, RuntimeServiceId> =
    nomos_da_sampling::DaSamplingService<
        KzgrsSamplingBackend,
        SamplingAdapter,
        nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        RuntimeServiceId,
    >;

pub type DaMempoolService<DaSamplingNetwork, RuntimeServiceId> = nomos_mempool::DaMempoolService<
    nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
        BlobInfo,
        <BlobInfo as DispersedBlobInfo>::BlobId,
        RuntimeServiceId,
    >,
    MockPool<HeaderId, BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
    KzgrsSamplingBackend,
    DaSamplingNetwork,
    nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire, DaStorageConverter>,
    RuntimeServiceId,
>;

pub type CryptarchiaService<SamplingAdapter, RuntimeServiceId> = CryptarchiaConsensus<
    chain_service::network::adapters::libp2p::LibP2pAdapter<
        SignedMantleTx,
        BlobInfo,
        RuntimeServiceId,
    >,
    BlendService<RuntimeServiceId>,
    MockPool<HeaderId, SignedMantleTx, <SignedMantleTx as Transaction>::Hash>,
    nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
        SignedMantleTx,
        <SignedMantleTx as Transaction>::Hash,
        RuntimeServiceId,
    >,
    MockPool<HeaderId, BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
    nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
        BlobInfo,
        <BlobInfo as DispersedBlobInfo>::BlobId,
        RuntimeServiceId,
    >,
    nomos_core::mantle::select::FillSize<MB16, SignedMantleTx>,
    nomos_core::da::blob::select::FillSize<MB16, BlobInfo>,
    RocksBackend<Wire>,
    KzgrsSamplingBackend,
    SamplingAdapter,
    nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire, DaStorageConverter>,
    NtpTimeBackend,
    RuntimeServiceId,
>;

pub type MembershipService<RuntimeServiceId> = nomos_membership::MembershipService<
    MembershipBackend,
    MembershipSdp<RuntimeServiceId>,
    RuntimeServiceId,
>;

pub type MembershipBackend = MockMembershipBackend;

pub type MembershipSdp<RuntimeServiceId> =
    LedgerSdpAdapter<MockSdpBackend, Metadata, RuntimeServiceId>;

pub type DaMembershipAdapter<RuntimeServiceId> = MembershipServiceAdapter<
    MockMembershipBackend,
    LedgerSdpAdapter<MockSdpBackend, Metadata, RuntimeServiceId>,
    RuntimeServiceId,
>;

pub type SdpService<RuntimeServiceId> =
    nomos_sdp::SdpService<MockSdpBackend, Metadata, RuntimeServiceId>;

pub type DaMembershipStorageGeneric<RuntimeServiceId> =
    RocksAdapter<RocksBackend<Wire>, RuntimeServiceId>;
