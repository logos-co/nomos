use cryptarchia_consensus::CryptarchiaConsensus;
use kzgrs_backend::{
    common::share::DaShare,
    dispersal::{BlobInfo, Metadata},
};
use nomos_core::{da::blob::info::DispersedBlobInfo, header::HeaderId, tx::Transaction};
use nomos_da_indexer::consensus::adapters::cryptarchia::CryptarchiaConsensusAdapter;
use nomos_da_network_service::membership::adapters::service::MembershipServiceAdapter;
use nomos_da_sampling::{
    api::http::HttApiAdapter, backend::kzgrs::KzgrsSamplingBackend,
    storage::adapters::rocksdb::converter::DaStorageConverter,
};
use nomos_da_verifier::backend::kzgrs::KzgrsDaVerifier;
use nomos_mantle_core::tx::SignedMantleTx;
use nomos_membership::{adapters::sdp::LedgerSdpAdapter, backends::mock::MockMembershipBackend};
use nomos_mempool::backend::mockpool::MockPool;
use nomos_sdp::adapters::{
    declaration::repository::LedgerDeclarationAdapter,
    services::services_repository::LedgerServicesAdapter,
};
use nomos_sdp_core::ledger::SdpLedger;
use nomos_storage::backends::rocksdb::RocksBackend;
use nomos_time::backends::NtpTimeBackend;
use rand_chacha::ChaCha20Rng;

use crate::{NomosDaMembership, Wire, MB16};

pub type TxMempoolService<RuntimeServiceId> = nomos_mempool::TxMempoolService<
    nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
        SignedMantleTx,
        <SignedMantleTx as Transaction>::Hash,
        RuntimeServiceId,
    >,
    MockPool<HeaderId, SignedMantleTx, <SignedMantleTx as Transaction>::Hash>,
    RuntimeServiceId,
>;

pub type TimeService<RuntimeServiceId> = nomos_time::TimeService<NtpTimeBackend, RuntimeServiceId>;

pub type DaIndexerService<SamplingAdapter, VerifierNetwork, RuntimeServiceId> =
    nomos_da_indexer::DataIndexerService<
        // Indexer specific.
        DaShare,
        nomos_da_indexer::storage::adapters::rocksdb::RocksAdapter<
            Wire,
            BlobInfo,
            DaStorageConverter,
        >,
        CryptarchiaConsensusAdapter<SignedMantleTx, BlobInfo>,
        // Cryptarchia specific, should be the same as in `Cryptarchia` type above.
        cryptarchia_consensus::network::adapters::libp2p::LibP2pAdapter<
            SignedMantleTx,
            BlobInfo,
            RuntimeServiceId,
        >,
        cryptarchia_consensus::blend::adapters::libp2p::LibP2pAdapter<
            nomos_blend_service::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
            SignedMantleTx,
            BlobInfo,
            RuntimeServiceId,
        >,
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
        nomos_core::tx::select::FillSize<MB16, SignedMantleTx>,
        nomos_core::da::blob::select::FillSize<MB16, BlobInfo>,
        RocksBackend<Wire>,
        KzgrsSamplingBackend<ChaCha20Rng>,
        SamplingAdapter,
        ChaCha20Rng,
        nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        KzgrsDaVerifier,
        VerifierNetwork,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        NtpTimeBackend,
        HttApiAdapter<NomosDaMembership>,
        RuntimeServiceId,
    >;

pub type DaVerifierService<VerifierAdapter, RuntimeServiceId> =
    nomos_da_verifier::DaVerifierService<
        KzgrsDaVerifier,
        VerifierAdapter,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        RuntimeServiceId,
    >;

pub type DaSamplingService<SamplingAdapter, VerifierNetworkAdapter, RuntimeServiceId> =
    nomos_da_sampling::DaSamplingService<
        KzgrsSamplingBackend<ChaCha20Rng>,
        SamplingAdapter,
        ChaCha20Rng,
        nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        KzgrsDaVerifier,
        VerifierNetworkAdapter,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        HttApiAdapter<NomosDaMembership>,
        RuntimeServiceId,
    >;

pub type DaMempoolService<DaSamplingNetwork, VerifierNetwork, RuntimeServiceId> =
    nomos_mempool::DaMempoolService<
        nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
            BlobInfo,
            <BlobInfo as DispersedBlobInfo>::BlobId,
            RuntimeServiceId,
        >,
        MockPool<HeaderId, BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
        KzgrsSamplingBackend<ChaCha20Rng>,
        DaSamplingNetwork,
        ChaCha20Rng,
        nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        KzgrsDaVerifier,
        VerifierNetwork,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        HttApiAdapter<NomosDaMembership>,
        RuntimeServiceId,
    >;

pub type CryptarchiaService<SamplingAdapter, VerifierNetwork, RuntimeServiceId> =
    CryptarchiaConsensus<
        cryptarchia_consensus::network::adapters::libp2p::LibP2pAdapter<
            SignedMantleTx,
            BlobInfo,
            RuntimeServiceId,
        >,
        cryptarchia_consensus::blend::adapters::libp2p::LibP2pAdapter<
            nomos_blend_service::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
            SignedMantleTx,
            BlobInfo,
            RuntimeServiceId,
        >,
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
        nomos_core::tx::select::FillSize<MB16, SignedMantleTx>,
        nomos_core::da::blob::select::FillSize<MB16, BlobInfo>,
        RocksBackend<Wire>,
        KzgrsSamplingBackend<ChaCha20Rng>,
        SamplingAdapter,
        ChaCha20Rng,
        nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        KzgrsDaVerifier,
        VerifierNetwork,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<
            DaShare,
            Wire,
            DaStorageConverter,
        >,
        NtpTimeBackend,
        HttApiAdapter<NomosDaMembership>,
        RuntimeServiceId,
    >;

pub type MembershipService<RuntimeServiceId> = nomos_membership::MembershipService<
    MockMembershipBackend,
    LedgerSdpAdapter<
        SdpLedger<LedgerDeclarationAdapter, LedgerServicesAdapter, Metadata>,
        LedgerDeclarationAdapter,
        LedgerServicesAdapter,
        Metadata,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub type DaMembershipAdapter<RuntimeServiceId> = MembershipServiceAdapter<
    MockMembershipBackend,
    LedgerSdpAdapter<
        SdpLedger<LedgerDeclarationAdapter, LedgerServicesAdapter, Metadata>,
        LedgerDeclarationAdapter,
        LedgerServicesAdapter,
        Metadata,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub type SdpService<RuntimeServiceId> = nomos_sdp::SdpService<
    SdpLedger<LedgerDeclarationAdapter, LedgerServicesAdapter, Metadata>,
    LedgerDeclarationAdapter,
    LedgerServicesAdapter,
    Metadata,
    RuntimeServiceId,
>;
