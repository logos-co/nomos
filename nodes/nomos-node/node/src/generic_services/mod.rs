use chain_leader::CryptarchiaLeader;
use chain_service::{CryptarchiaConsensus, network::adapters::libp2p::LibP2pAdapter};
use kzgrs_backend::common::share::DaShare;
use nomos_core::{
    header::HeaderId,
    mantle::{SignedMantleTx, Transaction, TxHash},
};
use nomos_da_network_service::{
    membership::adapters::service::MembershipServiceAdapter,
    sdp::adapters::sdp_service::SdpServiceAdapter, storage::adapters::rocksdb::RocksAdapter,
};
use nomos_da_sampling::{
    backend::kzgrs::KzgrsSamplingBackend, storage::adapters::rocksdb::converter::DaStorageConverter,
};
use nomos_da_verifier::{
    backend::kzgrs::KzgrsDaVerifier, mempool::kzgrs::KzgrsMempoolNetworkAdapter,
};
use nomos_membership_service::{
    adapters::sdp::ledger::LedgerSdpAdapter, backends::membership::PersistentMembershipBackend,
};
use nomos_sdp::backends::mock::MockSdpBackend;
use nomos_storage::backends::rocksdb::RocksBackend;
use nomos_time::backends::NtpTimeBackend;
use tx_service::{backend::pool::Mempool, storage::adapters::rocksdb::RocksStorageAdapter};

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
    Mempool<
        HeaderId,
        SignedMantleTx,
        TxHash,
        RocksStorageAdapter<SignedMantleTx, <SignedMantleTx as Transaction>::Hash>,
        RuntimeServiceId,
    >,
    RocksStorageAdapter<SignedMantleTx, <SignedMantleTx as Transaction>::Hash>,
    RuntimeServiceId,
>;

pub type TimeService<RuntimeServiceId> = nomos_time::TimeService<NtpTimeBackend, RuntimeServiceId>;

pub type VerifierMempoolAdapter<NetworkAdapter, RuntimeServiceId> = KzgrsMempoolNetworkAdapter<
    tx_service::network::adapters::libp2p::Libp2pAdapter<SignedMantleTx, TxHash, RuntimeServiceId>,
    Mempool<
        HeaderId,
        SignedMantleTx,
        TxHash,
        RocksStorageAdapter<SignedMantleTx, <SignedMantleTx as Transaction>::Hash>,
        RuntimeServiceId,
    >,
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

pub type MempoolAdapter<RuntimeServiceId> = tx_service::network::adapters::libp2p::Libp2pAdapter<
    SignedMantleTx,
    <SignedMantleTx as Transaction>::Hash,
    RuntimeServiceId,
>;

pub type MempoolBackend<RuntimeServiceId> = Mempool<
    HeaderId,
    SignedMantleTx,
    <SignedMantleTx as Transaction>::Hash,
    RocksStorageAdapter<SignedMantleTx, <SignedMantleTx as Transaction>::Hash>,
    RuntimeServiceId,
>;

pub type CryptarchiaService<SamplingAdapter, RuntimeServiceId> = CryptarchiaConsensus<
    LibP2pAdapter<SignedMantleTx, RuntimeServiceId>,
    MempoolBackend<RuntimeServiceId>,
    MempoolAdapter<RuntimeServiceId>,
    RocksBackend,
    KzgrsSamplingBackend,
    SamplingAdapter,
    DaSamplingStorage,
    NtpTimeBackend,
    RuntimeServiceId,
>;

pub type WalletService<Cryptarchia, RuntimeServiceId> =
    nomos_wallet::WalletService<Cryptarchia, SignedMantleTx, RocksBackend, RuntimeServiceId>;

pub type CryptarchiaLeaderService<Cryptarchia, Wallet, SamplingAdapter, RuntimeServiceId> =
    CryptarchiaLeader<
        BlendService<SamplingAdapter, RuntimeServiceId>,
        MempoolBackend<RuntimeServiceId>,
        MempoolAdapter<RuntimeServiceId>,
        nomos_core::mantle::select::FillSize<MB16, SignedMantleTx>,
        KzgrsSamplingBackend,
        SamplingAdapter,
        DaSamplingStorage,
        NtpTimeBackend,
        Cryptarchia,
        Wallet,
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

pub type MembershipSdp<RuntimeServiceId> = LedgerSdpAdapter<MockSdpBackend, RuntimeServiceId>;

pub type DaMembershipAdapter<RuntimeServiceId> = MembershipServiceAdapter<
    MembershipBackend<RuntimeServiceId>,
    LedgerSdpAdapter<MockSdpBackend, RuntimeServiceId>,
    MembershipStorageGeneric<RuntimeServiceId>,
    RuntimeServiceId,
>;

pub type SdpService<RuntimeServiceId> = nomos_sdp::SdpService<MockSdpBackend, RuntimeServiceId>;
pub type SdpServiceAdapterGeneric<RuntimeServiceId> =
    SdpServiceAdapter<MockSdpBackend, RuntimeServiceId>;

pub type DaMembershipStorageGeneric<RuntimeServiceId> =
    RocksAdapter<RocksBackend, RuntimeServiceId>;
