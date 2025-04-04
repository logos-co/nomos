pub mod api;
pub mod config;

use api::backend::AxumBackend;
use kzgrs_backend::common::share::DaShare;
use nomos_blend_service::{
    backends::libp2p::Libp2pBlendBackend as BlendBackend,
    network::libp2p::Libp2pAdapter as BlendNetworkAdapter,
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
    BlobInfo, HeaderId, MempoolNetworkAdapter, NetworkBackend, NomosDaMembership, RocksBackend,
    SystemSig, Tx, Wire, MB16,
};
use nomos_time::backends::NtpTimeBackend;
use overwatch::derive_services;
use rand_chacha::ChaCha20Rng;

#[cfg(feature = "tracing")]
pub(crate) type TracingService = Tracing<RuntimeServiceId>;

pub(crate) type NetworkService = nomos_network::NetworkService<NetworkBackend, RuntimeServiceId>;

pub(crate) type BlendService = nomos_blend_service::BlendService<
    BlendBackend,
    BlendNetworkAdapter<RuntimeServiceId>,
    RuntimeServiceId,
>;

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
pub(crate) type DaDispersalService = DispersalService<
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

pub(crate) type DaIndexerService = nomos_node::generic_services::DaIndexerService<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        RuntimeServiceId,
    >,
    VerifierNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
    RuntimeServiceId,
>;

pub(crate) type DaVerifierService = nomos_node::generic_services::DaVerifierService<
    VerifierNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
    RuntimeServiceId,
>;

pub(crate) type DaSamplingService = nomos_node::generic_services::DaSamplingService<
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

pub(crate) type DaNetworkService = nomos_da_network_service::NetworkService<
    DaNetworkExecutorBackend<NomosDaMembership>,
    RuntimeServiceId,
>;

pub(crate) type ClMempoolService = nomos_node::generic_services::TxMempoolService<RuntimeServiceId>;

pub(crate) type DaMempoolService = nomos_node::generic_services::DaMempoolService<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        RuntimeServiceId,
    >,
    VerifierNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
    RuntimeServiceId,
>;

pub(crate) type CryptarchiaService = nomos_node::generic_services::CryptarchiaService<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        RuntimeServiceId,
    >,
    VerifierNetworkAdapter<NomosDaMembership, RuntimeServiceId>,
    RuntimeServiceId,
>;

pub(crate) type TimeService = nomos_node::generic_services::TimeService<RuntimeServiceId>;

pub(crate) type ApiService = nomos_api::ApiService<
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
        NtpTimeBackend,
        HttApiAdapter<NomosDaMembership>,
        MB16,
    >,
    RuntimeServiceId,
>;

pub(crate) type StorageService =
    nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>;

pub(crate) type SystemSigService = SystemSig<RuntimeServiceId>;

#[derive_services]
pub struct NomosExecutor {
    #[cfg(feature = "tracing")]
    tracing: TracingService,
    network: NetworkService,
    blend: BlendService,
    da_dispersal: DaDispersalService,
    da_indexer: DaIndexerService,
    da_verifier: DaVerifierService,
    da_sampling: DaSamplingService,
    da_network: DaNetworkService,
    cl_mempool: ClMempoolService,
    da_mempool: DaMempoolService,
    cryptarchia: CryptarchiaService,
    time: TimeService,
    http: ApiService,
    storage: StorageService,
    system_sig: SystemSigService,
}
