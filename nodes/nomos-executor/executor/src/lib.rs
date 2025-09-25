pub mod api;
pub mod config;

use api::backend::AxumBackend;
use kzgrs_backend::common::share::DaShare;
use nomos_core::mantle::SignedMantleTx;
use nomos_da_dispersal::{
    DispersalService,
    adapters::{
        network::libp2p::Libp2pNetworkAdapter as DispersalNetworkAdapter,
        wallet::mock::MockWalletAdapter as DispersalWalletAdapter,
    },
    backend::kzgrs::DispersalKZGRSBackend,
};
use nomos_da_network_service::backends::libp2p::executor::DaNetworkExecutorBackend;
use nomos_da_sampling::{
    backend::kzgrs::KzgrsSamplingBackend,
    storage::adapters::rocksdb::{
        RocksAdapter as SamplingStorageAdapter, converter::DaStorageConverter,
    },
};
use nomos_da_verifier::{
    backend::kzgrs::KzgrsDaVerifier,
    network::adapters::executor::Libp2pAdapter as VerifierNetworkAdapter,
    storage::adapters::rocksdb::RocksAdapter as VerifierStorageAdapter,
};
#[cfg(feature = "tracing")]
use nomos_node::Tracing;
use nomos_node::{
    BlobInfo, DaNetworkApiAdapter, MB16, NetworkBackend, NomosDaMembership, RocksBackend,
    SystemSig,
    generic_services::{
        DaMembershipAdapter, DaMembershipStorageGeneric, MembershipService, SdpService,
        VerifierMempoolAdapter,
        blend::{BlendProofsGenerator, BlendProofsVerifier},
    },
};
use nomos_time::backends::NtpTimeBackend;
use overwatch::derive_services;

#[cfg(feature = "tracing")]
pub(crate) type TracingService = Tracing<RuntimeServiceId>;

type DaMembershipStorage = DaMembershipStorageGeneric<RuntimeServiceId>;

pub(crate) type NetworkService = nomos_network::NetworkService<NetworkBackend, RuntimeServiceId>;

pub(crate) type BlendCoreService =
    nomos_node::generic_services::blend::BlendCoreService<RuntimeServiceId>;
pub(crate) type BlendEdgeService =
    nomos_node::generic_services::blend::BlendEdgeService<RuntimeServiceId>;
pub(crate) type BlendService = nomos_node::generic_services::blend::BlendService<RuntimeServiceId>;

pub(crate) type BlockBroadcastService = broadcast_service::BlockBroadcastService<RuntimeServiceId>;

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

pub(crate) type ApiStorageAdapter<RuntimeServiceId> =
    nomos_api::http::storage::adapters::rocksdb::RocksAdapter<RuntimeServiceId>;

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
        VerifierStorageAdapter<DaShare, DaStorageConverter>,
        SignedMantleTx,
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
        SamplingStorageAdapter<DaShare, DaStorageConverter>,
        VerifierMempoolAdapter<DaNetworkAdapter, RuntimeServiceId>,
        NtpTimeBackend,
        DaNetworkApiAdapter,
        ApiStorageAdapter<RuntimeServiceId>,
        BlendProofsGenerator,
        BlendProofsVerifier,
        MB16,
    >,
    RuntimeServiceId,
>;

pub(crate) type StorageService = nomos_storage::StorageService<RocksBackend, RuntimeServiceId>;

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
    da_verifier: DaVerifierService,
    da_sampling: DaSamplingService,
    da_network: DaNetworkService,
    membership: MembershipService<RuntimeServiceId>,
    sdp: SdpService<RuntimeServiceId>,
    cl_mempool: ClMempoolService,
    cryptarchia: CryptarchiaService,
    block_broadcast: BlockBroadcastService,
    time: TimeService,
    http: ApiService,
    storage: StorageService,
    system_sig: SystemSigService,
    #[cfg(feature = "testing")]
    testing_http: TestingApiService<RuntimeServiceId>,
}
