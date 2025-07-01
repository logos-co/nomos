pub mod api;
pub mod config;

use api::backend::AxumBackend;
use kzgrs_backend::common::share::DaShare;
use nomos_blend_service::{
    backends::libp2p::Libp2pBlendBackend as BlendBackend,
    network::libp2p::Libp2pAdapter as BlendNetworkAdapter,
};
use nomos_core::{da::blob::info::DispersedBlobInfo, mantle::SignedMantleTx};
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
    api::http::HttApiAdapter,
    backend::kzgrs::KzgrsSamplingBackend,
    storage::adapters::rocksdb::{
        converter::DaStorageConverter, RocksAdapter as SamplingStorageAdapter,
    },
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
    generic_services::{DaMembershipAdapter, MembershipService, SdpService},
    BlobInfo, DaMembershipStorage, HeaderId, MempoolNetworkAdapter, NetworkBackend,
    NomosDaMembership, RocksBackend, SystemSig, Wire, MB16,
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
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    ChaCha20Rng,
    SamplingStorageAdapter<DaShare, Wire, DaStorageConverter>,
    KzgrsDaVerifier,
    VerifierNetworkAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    VerifierStorageAdapter<DaShare, Wire, DaStorageConverter>,
    HttApiAdapter<NomosDaMembership>,
    RuntimeServiceId,
>;
pub(crate) type DaDispersalService = DispersalService<
    DispersalKZGRSBackend<
        DispersalNetworkAdapter<
            NomosDaMembership,
            DaMembershipAdapter<RuntimeServiceId>,
            DaMembershipStorage,
            RuntimeServiceId,
        >,
        DispersalMempoolAdapter,
    >,
    DispersalNetworkAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    DispersalMempoolAdapter,
    NomosDaMembership,
    kzgrs_backend::dispersal::Metadata,
    RuntimeServiceId,
>;

pub(crate) type DaIndexerService = nomos_node::generic_services::DaIndexerService<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    VerifierNetworkAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type DaVerifierService = nomos_node::generic_services::DaVerifierService<
    VerifierNetworkAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type DaSamplingService = nomos_node::generic_services::DaSamplingService<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    nomos_da_verifier::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type DaNetworkService = nomos_da_network_service::NetworkService<
    DaNetworkExecutorBackend<NomosDaMembership>,
    NomosDaMembership,
    DaMembershipAdapter<RuntimeServiceId>,
    DaMembershipStorage,
    RuntimeServiceId,
>;

pub(crate) type ClMempoolService = nomos_node::generic_services::TxMempoolService<RuntimeServiceId>;

pub(crate) type DaMempoolService = nomos_node::generic_services::DaMempoolService<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    VerifierNetworkAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type CryptarchiaService = nomos_node::generic_services::CryptarchiaService<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    VerifierNetworkAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type TimeService = nomos_node::generic_services::TimeService<RuntimeServiceId>;

pub(crate) type ApiStorageAdapter<StorageOp, RuntimeServiceId> =
    nomos_api::http::storage::adapters::rocksdb::RocksAdapter<StorageOp, RuntimeServiceId>;

pub(crate) type ApiService = nomos_api::ApiService<
    AxumBackend<
        (),
        DaShare,
        BlobInfo,
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        BlobInfo,
        KzgrsDaVerifier,
        VerifierNetworkAdapter<
            NomosDaMembership,
            DaMembershipAdapter<RuntimeServiceId>,
            DaMembershipStorage,
            RuntimeServiceId,
        >,
        VerifierStorageAdapter<DaShare, Wire, DaStorageConverter>,
        SignedMantleTx,
        Wire,
        DaStorageConverter,
        DispersalKZGRSBackend<
            DispersalNetworkAdapter<
                NomosDaMembership,
                DaMembershipAdapter<RuntimeServiceId>,
                DaMembershipStorage,
                RuntimeServiceId,
            >,
            DispersalMempoolAdapter,
        >,
        DispersalNetworkAdapter<
            NomosDaMembership,
            DaMembershipAdapter<RuntimeServiceId>,
            DaMembershipStorage,
            RuntimeServiceId,
        >,
        DispersalMempoolAdapter,
        kzgrs_backend::dispersal::Metadata,
        KzgrsSamplingBackend<ChaCha20Rng>,
        nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
            NomosDaMembership,
            DaMembershipAdapter<RuntimeServiceId>,
            DaMembershipStorage,
            RuntimeServiceId,
        >,
        ChaCha20Rng,
        SamplingStorageAdapter<DaShare, Wire, DaStorageConverter>,
        NtpTimeBackend,
        HttApiAdapter<NomosDaMembership>,
        ApiStorageAdapter<Wire, RuntimeServiceId>,
        MB16,
    >,
    RuntimeServiceId,
>;

pub(crate) type StorageService =
    nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>;

pub(crate) type SystemSigService = SystemSig<RuntimeServiceId>;

#[cfg(feature = "testing")]
type TestingApiService<RuntimeServiceId> =
    nomos_api::ApiService<api::testing::backend::TestAxumBackend, RuntimeServiceId>;

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
    membership: MembershipService<RuntimeServiceId>,
    sdp: SdpService<RuntimeServiceId>,
    cl_mempool: ClMempoolService,
    da_mempool: DaMempoolService,
    cryptarchia: CryptarchiaService,
    time: TimeService,
    http: ApiService,
    storage: StorageService,
    system_sig: SystemSigService,

    #[cfg(feature = "testing")]
    testing_http: TestingApiService<RuntimeServiceId>,
}
