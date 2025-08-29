pub mod api;
pub mod config;
pub mod generic_services;

use bytes::Bytes;
use color_eyre::eyre::Result;
use generic_services::VerifierMempoolAdapter;
use kzgrs_backend::common::share::DaShare;
pub use kzgrs_backend::dispersal::BlobInfo;
pub use nomos_blend_service::{
    core::{
        backends::libp2p::Libp2pBlendBackend as BlendBackend,
        network::libp2p::Libp2pAdapter as BlendNetworkAdapter,
    },
    membership::service::Adapter as BlendMembershipAdapter,
};
use nomos_core::mantle::SignedMantleTx;
pub use nomos_core::{
    da::blob::{info::DispersedBlobInfo, select::FillSize as FillSizeWithBlobs},
    header::HeaderId,
    mantle::{select::FillSize as FillSizeWithTx, Transaction},
    wire,
};
pub use nomos_da_network_service::backends::libp2p::validator::DaNetworkValidatorBackend;
use nomos_da_network_service::{
    api::http::HttApiAdapter, membership::handler::DaMembershipHandler, DaAddressbook,
};
use nomos_da_sampling::{
    backend::kzgrs::KzgrsSamplingBackend,
    network::adapters::validator::Libp2pAdapter as SamplingLibp2pAdapter,
    storage::adapters::rocksdb::{
        converter::DaStorageConverter, RocksAdapter as SamplingStorageAdapter,
    },
};
use nomos_da_verifier::{
    backend::kzgrs::KzgrsDaVerifier,
    network::adapters::validator::Libp2pAdapter as VerifierNetworkAdapter,
    storage::adapters::rocksdb::RocksAdapter as VerifierStorageAdapter,
};
use nomos_libp2p::PeerId;
pub use nomos_mempool::{
    da::settings::DaMempoolSettings,
    network::adapters::libp2p::{
        Libp2pAdapter as MempoolNetworkAdapter, Settings as MempoolAdapterSettings,
    },
};
pub use nomos_network::backends::libp2p::Libp2p as NetworkBackend;
pub use nomos_storage::backends::{
    rocksdb::{RocksBackend, RocksBackendSettings},
    StorageSerde,
};
pub use nomos_system_sig::SystemSig;
use nomos_time::backends::NtpTimeBackend;
#[cfg(feature = "tracing")]
pub use nomos_tracing_service::Tracing;
use overwatch::derive_services;
use serde::{de::DeserializeOwned, Serialize};
use subnetworks_assignations::versions::history_aware_refill::HistoryAware;

pub use crate::config::{Config, CryptarchiaArgs, HttpArgs, LogArgs, NetworkArgs};
use crate::{
    api::backend::AxumBackend,
    generic_services::{
        DaMembershipAdapter, DaMembershipStorageGeneric, MembershipService, SdpService,
    },
};

pub const CONSENSUS_TOPIC: &str = "/cryptarchia/proto";
pub const CL_TOPIC: &str = "cl";
pub const DA_TOPIC: &str = "da";
pub const MB16: usize = 1024 * 1024 * 16;

pub struct Wire;

impl StorageSerde for Wire {
    type Error = wire::Error;

    fn serialize<T: Serialize>(value: T) -> Bytes {
        wire::serialize(&value).unwrap().into()
    }

    fn deserialize<T: DeserializeOwned>(buff: Bytes) -> Result<T, Self::Error> {
        wire::deserialize(&buff)
    }
}

/// Membership used by the DA Network service.
pub type NomosDaMembership = HistoryAware<PeerId>;
type DaMembershipStorage = DaMembershipStorageGeneric<RuntimeServiceId>;
pub type DaNetworkApiAdapter = HttApiAdapter<DaMembershipHandler<NomosDaMembership>, DaAddressbook>;

#[cfg(feature = "tracing")]
pub(crate) type TracingService = Tracing<RuntimeServiceId>;

pub(crate) type NetworkService = nomos_network::NetworkService<NetworkBackend, RuntimeServiceId>;

pub(crate) type BlendCoreService = nomos_blend_service::core::BlendService<
    BlendBackend,
    PeerId,
    BlendNetworkAdapter<RuntimeServiceId>,
    BlendMembershipAdapter<MembershipService<RuntimeServiceId>, PeerId>,
    RuntimeServiceId,
>;

pub(crate) type BlendEdgeService = nomos_blend_service::edge::BlendService<
    nomos_blend_service::edge::backends::libp2p::Libp2pBlendBackend,
    PeerId,
    <BlendNetworkAdapter<RuntimeServiceId> as nomos_blend_service::core::network::NetworkAdapter<
        RuntimeServiceId,
    >>::BroadcastSettings,
    BlendMembershipAdapter<MembershipService<RuntimeServiceId>, PeerId>,
    RuntimeServiceId,
>;

pub(crate) type BlendService =
    nomos_blend_service::BlendService<BlendCoreService, BlendEdgeService, RuntimeServiceId>;

pub(crate) type DaVerifierService = generic_services::DaVerifierService<
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

pub(crate) type DaSamplingService = generic_services::DaSamplingService<
    SamplingLibp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        DaNetworkApiAdapter,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type DaNetworkService = nomos_da_network_service::NetworkService<
    DaNetworkValidatorBackend<NomosDaMembership>,
    NomosDaMembership,
    DaMembershipAdapter<RuntimeServiceId>,
    DaMembershipStorage,
    DaNetworkApiAdapter,
    RuntimeServiceId,
>;

pub(crate) type ClMempoolService = generic_services::TxMempoolService<
    SamplingLibp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        DaNetworkApiAdapter,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type DaNetworkAdapter = nomos_da_sampling::network::adapters::validator::Libp2pAdapter<
    NomosDaMembership,
    DaMembershipAdapter<RuntimeServiceId>,
    DaMembershipStorage,
    DaNetworkApiAdapter,
    RuntimeServiceId,
>;

pub(crate) type DaMempoolService =
    generic_services::DaMempoolService<DaNetworkAdapter, RuntimeServiceId>;

pub(crate) type CryptarchiaService = generic_services::CryptarchiaService<
    nomos_da_sampling::network::adapters::validator::Libp2pAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        DaNetworkApiAdapter,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type TimeService = generic_services::TimeService<RuntimeServiceId>;

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
        KzgrsSamplingBackend,
        nomos_da_sampling::network::adapters::validator::Libp2pAdapter<
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

type StorageService = nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>;

type SystemSigService = SystemSig<RuntimeServiceId>;

#[cfg(feature = "testing")]
type TestingApiService<RuntimeServiceId> =
    nomos_api::ApiService<api::testing::backend::TestAxumBackend, RuntimeServiceId>;

#[derive_services]
pub struct Nomos {
    #[cfg(feature = "tracing")]
    tracing: TracingService,
    network: NetworkService,
    blend: BlendService,
    blend_core: BlendCoreService,
    blend_edge: BlendEdgeService,
    da_verifier: DaVerifierService,
    da_sampling: DaSamplingService,
    da_network: DaNetworkService,
    da_mempool: DaMempoolService,
    cl_mempool: ClMempoolService,
    cryptarchia: CryptarchiaService,
    membership: MembershipService<RuntimeServiceId>,
    sdp: SdpService<RuntimeServiceId>,
    time: TimeService,
    http: ApiService,
    storage: StorageService,
    system_sig: SystemSigService,
    #[cfg(feature = "testing")]
    testing_http: TestingApiService<RuntimeServiceId>,
}
