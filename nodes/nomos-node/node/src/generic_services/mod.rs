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
use nomos_membership::{
    adapters::sdp::ledger::LedgerSdpAdapter, backends::membership::PersistentMembershipBackend,
};
use nomos_mempool::backend::mockpool::MockPool;
use nomos_sdp::backends::mock::MockSdpBackend;
use nomos_storage::backends::rocksdb::RocksBackend;
use nomos_time::backends::NtpTimeBackend;

use crate::{generic_services::blend::BlendService, MB16};

pub mod blend;

pub type TxMempoolService<SamplingNetworkAdapter, RuntimeServiceId> =
    nomos_mempool::TxMempoolService<
        nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
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
    nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
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

pub type DaSamplingService<SamplingAdapter, RuntimeServiceId> =
    nomos_da_sampling::DaSamplingService<
        KzgrsSamplingBackend,
        SamplingAdapter,
        nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, DaStorageConverter>,
        RuntimeServiceId,
    >;

pub type CryptarchiaService<SamplingAdapter, RuntimeServiceId> = CryptarchiaConsensus<
    chain_service::network::adapters::libp2p::LibP2pAdapter<SignedMantleTx, RuntimeServiceId>,
    BlendService<RuntimeServiceId>,
    MockPool<HeaderId, SignedMantleTx, <SignedMantleTx as Transaction>::Hash>,
    nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
        SignedMantleTx,
        <SignedMantleTx as Transaction>::Hash,
        RuntimeServiceId,
    >,
    nomos_core::mantle::select::FillSize<MB16, SignedMantleTx>,
    RocksBackend,
    KzgrsSamplingBackend,
    SamplingAdapter,
    nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, DaStorageConverter>,
    NtpTimeBackend,
    RuntimeServiceId,
>;

pub type MembershipStorageGeneric<RuntimeServiceId> =
    nomos_membership::adapters::storage::rocksdb::MembershipRocksAdapter<
        RocksBackend,
        RuntimeServiceId,
    >;

pub type MembershipBackend<RuntimeServiceId> =
    PersistentMembershipBackend<MembershipStorageGeneric<RuntimeServiceId>>;

pub type MembershipService<RuntimeServiceId> = nomos_membership::MembershipService<
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
