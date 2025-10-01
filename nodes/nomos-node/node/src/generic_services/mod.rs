use chain_leader::CryptarchiaLeader;
use chain_service::CryptarchiaConsensus;
use kzgrs_backend::{common::share::DaShare, dispersal::Metadata};
use nomos_core::{
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
use nomos_membership_service::{
    adapters::sdp::ledger::LedgerSdpAdapter, backends::membership::PersistentMembershipBackend,
};
use nomos_sdp::backends::mock::MockSdpBackend;
use nomos_storage::backends::rocksdb::RocksBackend;
use nomos_time::backends::NtpTimeBackend;
use tx_service::backend::mockpool::MockPool;

use crate::{MB16, generic_services::blend::BlendService};

pub mod blend;

pub type TxMempoolService<SamplingNetworkAdapter, RuntimeServiceId> = tx_service::TxMempoolService<
    tx_service::network::adapters::libp2p::Libp2pAdapter<
        SignedMantleTx,
        <SignedMantleTx as Transaction>::Hash,
        RuntimeServiceId,
    >,
    SamplingNetworkAdapter,
    nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, DaStorageConverter>,
    MockPool<HeaderId, SignedMantleTx, <SignedMantleTx as Transaction>::Hash>,
    RuntimeServiceId,
>;

pub type TimeService<RuntimeServiceId> = nomos_time::TimeService<NtpTimeBackend, RuntimeServiceId>;

pub type VerifierMempoolAdapter<NetworkAdapter, RuntimeServiceId> = KzgrsMempoolAdapter<
    tx_service::network::adapters::libp2p::Libp2pAdapter<
        SignedMantleTx,
        <SignedMantleTx as Transaction>::Hash,
        RuntimeServiceId,
    >,
    MockPool<HeaderId, SignedMantleTx, <SignedMantleTx as Transaction>::Hash>,
    KzgrsSamplingBackend,
    NetworkAdapter,
    nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, DaStorageConverter>,
    RuntimeServiceId,
>;

pub type DaVerifierService<VerifierAdapter, MempoolAdapter, RuntimeServiceId> =
    nomos_da_verifier::DaVerifierService<
        KzgrsDaVerifier,
        VerifierAdapter,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<DaShare, DaStorageConverter>,
        MempoolAdapter,
        RuntimeServiceId,
    >;

pub type DaSamplingStorage =
    nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, DaStorageConverter>;

pub type DaSamplingService<SamplingAdapter, RuntimeServiceId> =
    nomos_da_sampling::DaSamplingService<
        KzgrsSamplingBackend,
        SamplingAdapter,
        DaSamplingStorage,
        RuntimeServiceId,
    >;

pub type Mempool = MockPool<HeaderId, SignedMantleTx, <SignedMantleTx as Transaction>::Hash>;
pub type MempoolAdapter<RuntimeServiceId> = tx_service::network::adapters::libp2p::Libp2pAdapter<
    SignedMantleTx,
    <SignedMantleTx as Transaction>::Hash,
    RuntimeServiceId,
>;

pub type CryptarchiaService<SamplingAdapter, RuntimeServiceId> = CryptarchiaConsensus<
    chain_service::network::adapters::libp2p::LibP2pAdapter<SignedMantleTx, RuntimeServiceId>,
    Mempool,
    MempoolAdapter<RuntimeServiceId>,
    RocksBackend,
    KzgrsSamplingBackend,
    SamplingAdapter,
    DaSamplingStorage,
    NtpTimeBackend,
    RuntimeServiceId,
>;

pub type CryptarchiaLeaderService<SamplingAdapter, RuntimeServiceId> = CryptarchiaLeader<
    BlendService<SamplingAdapter, RuntimeServiceId>,
    Mempool,
    MempoolAdapter<RuntimeServiceId>,
    nomos_core::mantle::select::FillSize<MB16, SignedMantleTx>,
    KzgrsSamplingBackend,
    SamplingAdapter,
    DaSamplingStorage,
    NtpTimeBackend,
    CryptarchiaService<SamplingAdapter, RuntimeServiceId>,
    RuntimeServiceId,
>;

pub type MembershipStorageGeneric<RuntimeServiceId> =
    nomos_membership_service::adapters::storage::rocksdb::MembershipRocksAdapter<
        RocksBackend,
        RuntimeServiceId,
    >;

pub type MembershipBackend<RuntimeServiceId> =
    PersistentMembershipBackend<MembershipStorageGeneric<RuntimeServiceId>>;

pub type MembershipService<RuntimeServiceId> = nomos_membership_service::MembershipService<
    MembershipBackend<RuntimeServiceId>,
    MembershipSdp<RuntimeServiceId>,
    MembershipStorageGeneric<RuntimeServiceId>,
    RuntimeServiceId,
>;

pub type MembershipSdp<RuntimeServiceId> =
    LedgerSdpAdapter<MockSdpBackend, Metadata, RuntimeServiceId>;

pub type DaMembershipAdapter<RuntimeServiceId> = MembershipServiceAdapter<
    MembershipBackend<RuntimeServiceId>,
    LedgerSdpAdapter<MockSdpBackend, Metadata, RuntimeServiceId>,
    MembershipStorageGeneric<RuntimeServiceId>,
    RuntimeServiceId,
>;

pub type SdpService<RuntimeServiceId> =
    nomos_sdp::SdpService<MockSdpBackend, Metadata, RuntimeServiceId>;

pub type DaMembershipStorageGeneric<RuntimeServiceId> =
    RocksAdapter<RocksBackend, RuntimeServiceId>;
