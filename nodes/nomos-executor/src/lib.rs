pub mod api;
// pub mod config;

use api::backend::AxumBackend;
use kzgrs_backend::common::share::DaShare;
use nomos_api::ApiService;
use nomos_blend_service::{
    backends::libp2p::Libp2pBlendBackend as BlendBackend,
    network::libp2p::Libp2pAdapter as BlendNetworkAdapter, BlendService,
};
use nomos_core::da::blob::info::DispersedBlobInfo;
use nomos_da_dispersal::{
    adapters::{
        mempool::kzgrs::KzgrsMempoolAdapter,
        network::libp2p::Libp2pNetworkAdapter as DispersalNetworkAdapter,
    },
    backend::kzgrs::DispersalKZGRSBackend,
    DispersalService,
};
use nomos_da_network_service::backends::libp2p::executor::DaNetworkExecutorBackend;
use nomos_da_sampling::{
    api::http::HttApiAdapter, backend::kzgrs::KzgrsSamplingBackend,
    storage::adapters::rocksdb::RocksAdapter as SamplingStorageAdapter,
};
use nomos_da_verifier::{
    backend::kzgrs::KzgrsDaVerifier,
    network::adapters::executor::Libp2pAdapter as VerifierNetworkAdapter,
    storage::adapters::rocksdb::RocksAdapter as VerifierStorageAdapter,
};
use nomos_mempool::backend::mockpool::MockPool;
#[cfg(feature = "tracing")]
use nomos_node::Tracing;
use nomos_node::{
    BlobInfo, Cryptarchia, DaIndexer, DaMempool, DaNetworkService, DaSampling, DaVerifier,
    HeaderId, MempoolNetworkAdapter, NetworkBackend, NetworkService, NomosDaMembership,
    NomosTimeService, RocksBackend, StorageService, SystemSig, Tx, TxMempool, Wire, MB16,
};
use overwatch::derive_services;
use rand_chacha::ChaCha20Rng;

#[cfg(feature = "tracing")]
type ExecutorTracingService = Tracing<RuntimeServiceId>;

type ExecutorNetworkService = NetworkService<NetworkBackend, RuntimeServiceId>;

type ExecutorBlendService =
    BlendService<BlendBackend, BlendNetworkAdapter<RuntimeServiceId>, RuntimeServiceId>;

type DispersalMempoolAdapter = KzgrsMempoolAdapter<
    MempoolNetworkAdapter<BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId, RuntimeServiceId>,
    MockPool<HeaderId, BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
    KzgrsSamplingBackend<ChaCha20Rng>,
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        RuntimeServiceId,
    >,
    ChaCha20Rng,
    SamplingStorageAdapter<DaShare, Wire>,
    KzgrsDaVerifier,
    VerifierNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
    VerifierStorageAdapter<DaShare, Wire>,
    HttApiAdapter<NomosDaMembership>,
    RuntimeServiceId,
>;
type ExecutorDaDispersalService = DispersalService<
    DispersalKZGRSBackend<
        DispersalNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
        DispersalMempoolAdapter,
    >,
    DispersalNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
    DispersalMempoolAdapter,
    NomosDaMembership,
    kzgrs_backend::dispersal::Metadata,
    RuntimeServiceId,
>;

type ExecutorDaIndexerService = DaIndexer<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        RuntimeServiceId,
    >,
    VerifierNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
    RuntimeServiceId,
>;

type ExecutorDaVerifierService =
    DaVerifier<VerifierNetworkAdapter<NomosDaMembership, RuntimeServiceId>, RuntimeServiceId>;

type ExecutorDaSamplingService = DaSampling<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        RuntimeServiceId,
    >,
    nomos_da_verifier::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

type ExecutorDaNetworkService =
    DaNetworkService<DaNetworkExecutorBackend<NomosDaMembership>, RuntimeServiceId>;

type ExecutorClMempoolService = TxMempool<RuntimeServiceId>;

type ExecutorDaMempoolService = DaMempool<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        RuntimeServiceId,
    >,
    VerifierNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
    RuntimeServiceId,
>;

type ExecutorCryptarchiaService = Cryptarchia<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        RuntimeServiceId,
    >,
    VerifierNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
    RuntimeServiceId,
>;

type ExecutorTimeService = NomosTimeService<RuntimeServiceId>;

type ExecutorApiService = ApiService<
    AxumBackend<
        (),
        DaShare,
        BlobInfo,
        NomosDaMembership,
        BlobInfo,
        KzgrsDaVerifier,
        VerifierNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
        VerifierStorageAdapter<DaShare, Wire>,
        Tx,
        Wire,
        DispersalKZGRSBackend<
            DispersalNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
            DispersalMempoolAdapter,
        >,
        DispersalNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
        DispersalMempoolAdapter,
        kzgrs_backend::dispersal::Metadata,
        KzgrsSamplingBackend<ChaCha20Rng>,
        nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
            NomosDaMembership,
            RuntimeServiceId,
        >,
        ChaCha20Rng,
        SamplingStorageAdapter<DaShare, Wire>,
        nomos_time::backends::system_time::SystemTimeBackend,
        HttApiAdapter<NomosDaMembership>,
        MB16,
    >,
    RuntimeServiceId,
>;

type ExecutorStorageService = StorageService<RocksBackend<Wire>, RuntimeServiceId>;

type ExecutorSystemSigService = SystemSig<RuntimeServiceId>;

#[derive_services]
pub struct NomosExecutor {
    #[cfg(feature = "tracing")]
    tracing: ExecutorTracingService,
    network: ExecutorNetworkService,
    blend: ExecutorBlendService,
    da_dispersal: ExecutorDaDispersalService,
    da_indexer: ExecutorDaIndexerService,
    da_verifier: ExecutorDaVerifierService,
    da_sampling: ExecutorDaSamplingService,
    da_network: ExecutorDaNetworkService,
    cl_mempool: ExecutorClMempoolService,
    da_mempool: ExecutorDaMempoolService,
    cryptarchia: ExecutorCryptarchiaService,
    time: ExecutorTimeService,
    http: ExecutorApiService,
    storage: ExecutorStorageService,
    system_sig: ExecutorSystemSigService,
}
