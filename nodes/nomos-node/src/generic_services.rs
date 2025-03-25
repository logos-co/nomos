use cryptarchia_consensus::CryptarchiaConsensus;
use kzgrs_backend::{common::share::DaShare, dispersal::BlobInfo};
use nomos_core::{da::blob::info::DispersedBlobInfo, header::HeaderId, tx::Transaction};
use nomos_da_indexer::consensus::adapters::cryptarchia::CryptarchiaConsensusAdapter;
use nomos_da_sampling::{api::http::HttApiAdapter, backend::kzgrs::KzgrsSamplingBackend};
use nomos_da_verifier::backend::kzgrs::KzgrsDaVerifier;
use nomos_mempool::backend::mockpool::MockPool;
use nomos_storage::backends::rocksdb::RocksBackend;
use nomos_time::backends::system_time::SystemTimeBackend;
use rand_chacha::ChaCha20Rng;

use crate::{NomosDaMembership, Tx, Wire, MB16};

pub type TxMempoolService<RuntimeServiceId> = nomos_mempool::TxMempoolService<
    nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
        Tx,
        <Tx as Transaction>::Hash,
        RuntimeServiceId,
    >,
    MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
    RuntimeServiceId,
>;

pub type TimeService<RuntimeServiceId> =
    nomos_time::TimeService<SystemTimeBackend, RuntimeServiceId>;

pub type DaIndexerService<SamplingAdapter, VerifierNetwork, RuntimeServiceId> =
    nomos_da_indexer::DataIndexerService<
        // Indexer specific.
        DaShare,
        nomos_da_indexer::storage::adapters::rocksdb::RocksAdapter<Wire, BlobInfo>,
        CryptarchiaConsensusAdapter<Tx, BlobInfo>,
        // Cryptarchia specific, should be the same as in `Cryptarchia` type above.
        cryptarchia_consensus::network::adapters::libp2p::LibP2pAdapter<
            Tx,
            BlobInfo,
            RuntimeServiceId,
        >,
        cryptarchia_consensus::blend::adapters::libp2p::LibP2pAdapter<
            nomos_blend_service::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
            Tx,
            BlobInfo,
            RuntimeServiceId,
        >,
        MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
        nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
            Tx,
            <Tx as Transaction>::Hash,
            RuntimeServiceId,
        >,
        MockPool<HeaderId, BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
        nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
            BlobInfo,
            <BlobInfo as DispersedBlobInfo>::BlobId,
            RuntimeServiceId,
        >,
        nomos_core::tx::select::FillSize<MB16, Tx>,
        nomos_core::da::blob::select::FillSize<MB16, BlobInfo>,
        RocksBackend<Wire>,
        KzgrsSamplingBackend<ChaCha20Rng>,
        SamplingAdapter,
        ChaCha20Rng,
        nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire>,
        KzgrsDaVerifier,
        VerifierNetwork,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire>,
        SystemTimeBackend,
        HttApiAdapter<NomosDaMembership>,
        RuntimeServiceId,
    >;

pub type DaVerifierService<VerifierAdapter, RuntimeServiceId> =
    nomos_da_verifier::DaVerifierService<
        KzgrsDaVerifier,
        VerifierAdapter,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire>,
        RuntimeServiceId,
    >;

pub type DaSamplingService<SamplingAdapter, VerifierNetworkAdapter, RuntimeServiceId> =
    nomos_da_sampling::DaSamplingService<
        KzgrsSamplingBackend<ChaCha20Rng>,
        SamplingAdapter,
        ChaCha20Rng,
        nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire>,
        KzgrsDaVerifier,
        VerifierNetworkAdapter,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire>,
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
        nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire>,
        KzgrsDaVerifier,
        VerifierNetwork,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire>,
        HttApiAdapter<NomosDaMembership>,
        RuntimeServiceId,
    >;

pub type CryptarchiaService<SamplingAdapter, VerifierNetwork, RuntimeServiceId> =
    CryptarchiaConsensus<
        cryptarchia_consensus::network::adapters::libp2p::LibP2pAdapter<
            Tx,
            BlobInfo,
            RuntimeServiceId,
        >,
        cryptarchia_consensus::blend::adapters::libp2p::LibP2pAdapter<
            nomos_blend_service::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
            Tx,
            BlobInfo,
            RuntimeServiceId,
        >,
        MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
        nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
            Tx,
            <Tx as Transaction>::Hash,
            RuntimeServiceId,
        >,
        MockPool<HeaderId, BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
        nomos_mempool::network::adapters::libp2p::Libp2pAdapter<
            BlobInfo,
            <BlobInfo as DispersedBlobInfo>::BlobId,
            RuntimeServiceId,
        >,
        nomos_core::tx::select::FillSize<MB16, Tx>,
        nomos_core::da::blob::select::FillSize<MB16, BlobInfo>,
        RocksBackend<Wire>,
        KzgrsSamplingBackend<ChaCha20Rng>,
        SamplingAdapter,
        ChaCha20Rng,
        nomos_da_sampling::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire>,
        KzgrsDaVerifier,
        VerifierNetwork,
        nomos_da_verifier::storage::adapters::rocksdb::RocksAdapter<DaShare, Wire>,
        SystemTimeBackend,
        HttApiAdapter<NomosDaMembership>,
        RuntimeServiceId,
    >;
