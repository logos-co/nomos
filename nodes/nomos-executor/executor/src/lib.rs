pub mod api;
pub mod config;

use api::backend::AxumBackend;
use kzgrs_backend::common::share::DaShare;
use nomos_blend_service::core::{
    backends::libp2p::Libp2pBlendBackend as BlendBackend,
    network::libp2p::Libp2pAdapter as BlendNetworkAdapter,
};
use nomos_core::mantle::SignedMantleTx;
use nomos_da_dispersal::{
    adapters::{
        network::libp2p::Libp2pNetworkAdapter as DispersalNetworkAdapter,
        wallet::mock::MockWalletAdapter as DispersalWalletAdapter,
    },
    backend::kzgrs::DispersalKZGRSBackend,
    DispersalService,
};
use nomos_da_network_service::backends::libp2p::executor::DaNetworkExecutorBackend;
use nomos_da_sampling::{
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
use nomos_libp2p::PeerId;
#[cfg(feature = "tracing")]
use nomos_node::Tracing;
use nomos_node::{
    generic_services::{
        DaMembershipAdapter, DaMembershipStorageGeneric, MembershipService, SdpService,
        VerifierMempoolAdapter,
    },
    BlobInfo, DaNetworkApiAdapter, NetworkBackend, NomosDaMembership, RocksBackend, SystemSig,
    Wire, MB16,
};
use nomos_time::backends::NtpTimeBackend;
use overwatch::derive_services;

#[cfg(feature = "tracing")]
pub(crate) type TracingService = Tracing<RuntimeServiceId>;

type DaMembershipStorage = DaMembershipStorageGeneric<RuntimeServiceId>;

pub(crate) type NetworkService = nomos_network::NetworkService<NetworkBackend, RuntimeServiceId>;

pub(crate) type BlendCoreService = nomos_blend_service::core::BlendService<
    BlendBackend,
    PeerId,
    BlendNetworkAdapter<RuntimeServiceId>,
    RuntimeServiceId,
>;

pub(crate) type BlendEdgeService = nomos_blend_service::edge::BlendService<
    nomos_blend_service::edge::backends::libp2p::Libp2pBlendBackend,
    PeerId,
    <BlendNetworkAdapter<RuntimeServiceId> as nomos_blend_service::core::network::NetworkAdapter<
        RuntimeServiceId,
    >>::BroadcastSettings,
    RuntimeServiceId,
>;

pub(crate) type BlendService =
    nomos_blend_service::BlendService<BlendCoreService, BlendEdgeService, RuntimeServiceId>;

pub(crate) type DaDispersalService = DispersalService<
    DispersalKZGRSBackend<
        DispersalNetworkAdapter<
            NomosDaMembership,
            DaMembershipAdapter<RuntimeServiceId>,
            DaMembershipStorage,
            DaNetworkApiAdapter,
            RuntimeServiceId,
        >,
        DispersalWalletAdapter,
    >,
    DispersalNetworkAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        DaNetworkApiAdapter,
        RuntimeServiceId,
    >,
    NomosDaMembership,
    RuntimeServiceId,
>;

pub(crate) type DaIndexerService = nomos_node::generic_services::DaIndexerService<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        DaNetworkApiAdapter,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type DaVerifierService = nomos_node::generic_services::DaVerifierService<
    VerifierNetworkAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        DaNetworkApiAdapter,
        RuntimeServiceId,
    >,
    VerifierMempoolAdapter<DaNetworkAdapter, RuntimeServiceId>,
    RuntimeServiceId,
>;

pub(crate) type DaSamplingService = nomos_node::generic_services::DaSamplingService<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        DaNetworkApiAdapter,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type DaNetworkService = nomos_da_network_service::NetworkService<
    DaNetworkExecutorBackend<NomosDaMembership>,
    NomosDaMembership,
    DaMembershipAdapter<RuntimeServiceId>,
    DaMembershipStorage,
    DaNetworkApiAdapter,
    RuntimeServiceId,
>;

pub(crate) type ClMempoolService = nomos_node::generic_services::TxMempoolService<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        DaNetworkApiAdapter,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type DaNetworkAdapter = nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
    NomosDaMembership,
    DaMembershipAdapter<RuntimeServiceId>,
    DaMembershipStorage,
    DaNetworkApiAdapter,
    RuntimeServiceId,
>;

pub(crate) type DaMempoolService =
    nomos_node::generic_services::DaMempoolService<DaNetworkAdapter, RuntimeServiceId>;

pub(crate) type CryptarchiaService = nomos_node::generic_services::CryptarchiaService<
    nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        DaNetworkApiAdapter,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type TimeService = nomos_node::generic_services::TimeService<RuntimeServiceId>;

pub(crate) type ApiStorageAdapter<StorageOp, RuntimeServiceId> =
    nomos_api::http::storage::adapters::rocksdb::RocksAdapter<StorageOp, RuntimeServiceId>;

pub(crate) type ApiService = nomos_api::ApiService<
    AxumBackend<
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
            DaNetworkApiAdapter,
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
                DaNetworkApiAdapter,
                RuntimeServiceId,
            >,
            DispersalWalletAdapter,
        >,
        DispersalNetworkAdapter<
            NomosDaMembership,
            DaMembershipAdapter<RuntimeServiceId>,
            DaMembershipStorage,
            DaNetworkApiAdapter,
            RuntimeServiceId,
        >,
        kzgrs_backend::dispersal::Metadata,
        KzgrsSamplingBackend,
        nomos_da_sampling::network::adapters::executor::Libp2pAdapter<
            NomosDaMembership,
            DaMembershipAdapter<RuntimeServiceId>,
            DaMembershipStorage,
            DaNetworkApiAdapter,
            RuntimeServiceId,
        >,
        SamplingStorageAdapter<DaShare, Wire, DaStorageConverter>,
        VerifierMempoolAdapter<DaNetworkAdapter, RuntimeServiceId>,
        NtpTimeBackend,
        DaNetworkApiAdapter,
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
    blend_core: BlendCoreService,
    blend_edge: BlendEdgeService,
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
