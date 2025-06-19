pub mod api;
pub mod config;
pub mod generic_services;

use bytes::Bytes;
use color_eyre::eyre::Result;
use kzgrs_backend::common::share::DaShare;
pub use kzgrs_backend::dispersal::BlobInfo;
pub use nomos_blend_service::{
    backends::libp2p::Libp2pBlendBackend as BlendBackend,
    network::libp2p::Libp2pAdapter as BlendNetworkAdapter,
};
use nomos_core::mantle::SignedMantleTx;
pub use nomos_core::{
    da::blob::{info::DispersedBlobInfo, select::FillSize as FillSizeWithBlobs},
    header::HeaderId,
    mantle::{select::FillSize as FillSizeWithTx, Transaction},
    wire,
};
pub use nomos_da_network_service::backends::libp2p::validator::DaNetworkValidatorBackend;
use nomos_da_network_service::storage::adapters::mock::MockStorage;
use nomos_da_sampling::{
    api::http::HttApiAdapter,
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
use rand_chacha::ChaCha20Rng;
use serde::{de::DeserializeOwned, Serialize};
use subnetworks_assignations::versions::v1::FillFromNodeList;

pub use crate::config::{Config, CryptarchiaArgs, HttpArgs, LogArgs, NetworkArgs};
use crate::{
    api::backend::AxumBackend,
    generic_services::{DaMembershipAdapter, MembershipService, SdpService},
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
pub type NomosDaMembership = FillFromNodeList;
pub type DaMembershipStorage = MockStorage;

#[cfg(feature = "tracing")]
pub(crate) type TracingService = Tracing<RuntimeServiceId>;

pub(crate) type NetworkService = nomos_network::NetworkService<NetworkBackend, RuntimeServiceId>;

pub(crate) type BlendService = nomos_blend_service::BlendService<
    BlendBackend,
    BlendNetworkAdapter<RuntimeServiceId>,
    RuntimeServiceId,
>;

pub(crate) type DaIndexerService = generic_services::DaIndexerService<
    nomos_da_sampling::network::adapters::validator::Libp2pAdapter<
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

pub(crate) type DaVerifierService = generic_services::DaVerifierService<
    VerifierNetworkAdapter<
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
        RuntimeServiceId,
    >,
    RuntimeServiceId,
>;

pub(crate) type DaSamplingService = generic_services::DaSamplingService<
    SamplingLibp2pAdapter<
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

pub(crate) type DaNetworkService = nomos_da_network_service::NetworkService<
    DaNetworkValidatorBackend<NomosDaMembership>,
    NomosDaMembership,
    DaMembershipAdapter<RuntimeServiceId>,
    DaMembershipStorage,
    RuntimeServiceId,
>;

pub(crate) type ClMempoolService = generic_services::TxMempoolService<RuntimeServiceId>;

pub(crate) type DaMempoolService = generic_services::DaMempoolService<
    nomos_da_sampling::network::adapters::validator::Libp2pAdapter<
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

pub(crate) type CryptarchiaService = generic_services::CryptarchiaService<
    nomos_da_sampling::network::adapters::validator::Libp2pAdapter<
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

pub(crate) type TimeService = generic_services::TimeService<RuntimeServiceId>;

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
        KzgrsSamplingBackend<ChaCha20Rng>,
        nomos_da_sampling::network::adapters::validator::Libp2pAdapter<
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

type StorageService = nomos_storage::StorageService<RocksBackend<Wire>, RuntimeServiceId>;

type SystemSigService = SystemSig<RuntimeServiceId>;

#[derive_services]
pub struct Nomos {
    #[cfg(feature = "tracing")]
    tracing: TracingService,
    network: NetworkService,
    blend: BlendService,
    da_indexer: DaIndexerService,
    da_verifier: DaVerifierService,
    da_sampling: DaSamplingService,
    da_network: DaNetworkService,
    cl_mempool: ClMempoolService,
    da_mempool: DaMempoolService,
    cryptarchia: CryptarchiaService,
    membership: MembershipService<RuntimeServiceId>,
    sdp: SdpService<RuntimeServiceId>,
    time: TimeService,
    http: ApiService,
    storage: StorageService,
    system_sig: SystemSigService,
}
