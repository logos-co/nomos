// pub mod api;
// pub mod config;
mod tx;

use bytes::Bytes;
use color_eyre::eyre::Result;
use kzgrs_backend::common::share::DaShare;
pub use kzgrs_backend::dispersal::BlobInfo;
use nomos_api::ApiService;
pub use nomos_blend_service::{
    backends::libp2p::Libp2pBlendBackend as BlendBackend,
    network::libp2p::Libp2pAdapter as BlendNetworkAdapter, BlendService,
};
pub use nomos_core::{
    da::blob::{info::DispersedBlobInfo, select::FillSize as FillSizeWithBlobs},
    header::HeaderId,
    tx::{select::FillSize as FillSizeWithTx, Transaction},
    wire,
};
use nomos_da_indexer::{
    consensus::adapters::cryptarchia::CryptarchiaConsensusAdapter,
    storage::adapters::rocksdb::RocksAdapter as IndexerStorageAdapter, DataIndexerService,
};
pub use nomos_da_network_service::{
    backends::libp2p::validator::DaNetworkValidatorBackend, NetworkService as DaNetworkService,
};
use nomos_da_sampling::{
    api::http::HttApiAdapter, backend::kzgrs::KzgrsSamplingBackend,
    network::adapters::validator::Libp2pAdapter as SamplingLibp2pAdapter,
    storage::adapters::rocksdb::RocksAdapter as SamplingStorageAdapter, DaSamplingService,
};
use nomos_da_verifier::{
    backend::kzgrs::KzgrsDaVerifier,
    network::adapters::validator::Libp2pAdapter as VerifierNetworkAdapter,
    storage::adapters::rocksdb::RocksAdapter as VerifierStorageAdapter, DaVerifierService,
};
use nomos_mempool::{backend::mockpool::MockPool, TxMempoolService};
pub use nomos_mempool::{
    da::{service::DaMempoolService, settings::DaMempoolSettings},
    network::adapters::libp2p::{
        Libp2pAdapter as MempoolNetworkAdapter, Settings as MempoolAdapterSettings,
    },
};
pub use nomos_network::{backends::libp2p::Libp2p as NetworkBackend, NetworkService};
pub use nomos_storage::{
    backends::{
        rocksdb::{RocksBackend, RocksBackendSettings},
        StorageSerde,
    },
    StorageService,
};
pub use nomos_system_sig::SystemSig;
use nomos_time::{backends::system_time::SystemTimeBackend, TimeService};
#[cfg(feature = "tracing")]
pub use nomos_tracing_service::Tracing;
use overwatch::{derive_services, OpaqueServiceHandle};
use overwatch_derive::Services;
use rand_chacha::ChaCha20Rng;
use serde::{de::DeserializeOwned, Serialize};
use subnetworks_assignations::versions::v1::FillFromNodeList;

// use crate::api::backend::AxumBackend;
// pub use crate::config::{Config, CryptarchiaArgs, HttpArgs, LogArgs,
// NetworkArgs}; pub use crate::tx::Tx;

/// Membership used by the DA Network service.
pub type NomosDaMembership = FillFromNodeList;

// pub type NomosApiService = ApiService<
//     AxumBackend<
//         (),
//         DaShare,
//         BlobInfo,
//         NomosDaMembership,
//         BlobInfo,
//         KzgrsDaVerifier,
//         VerifierNetworkAdapter<NomosDaMembership>,
//         VerifierStorageAdapter<DaShare, Wire>,
//         Tx,
//         Wire,
//         KzgrsSamplingBackend<ChaCha20Rng>,
//         nomos_da_sampling::network::adapters::validator::Libp2pAdapter<NomosDaMembership>,
//         ChaCha20Rng,
//         SamplingStorageAdapter<DaShare, Wire>,
//         SystemTimeBackend,
//         HttApiAdapter<NomosDaMembership>,
//         MB16,
//     >,
// >;

// pub const CONSENSUS_TOPIC: &str = "/cryptarchia/proto";
// pub const CL_TOPIC: &str = "cl";
// pub const DA_TOPIC: &str = "da";
// pub const MB16: usize = 1024 * 1024 * 16;

// pub type Cryptarchia<SamplingAdapter> =
// cryptarchia_consensus::CryptarchiaConsensus<
//     cryptarchia_consensus::network::adapters::libp2p::LibP2pAdapter<Tx,
// BlobInfo>,     cryptarchia_consensus::blend::adapters::libp2p::LibP2pAdapter<
//         BlendNetworkAdapter,
//         Tx,
//         BlobInfo,
//     >,
//     MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
//     MempoolNetworkAdapter<Tx, <Tx as Transaction>::Hash>,
//     MockPool<HeaderId, BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
//     MempoolNetworkAdapter<BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
//     FillSizeWithTx<MB16, Tx>,
//     FillSizeWithBlobs<MB16, BlobInfo>,
//     RocksBackend<Wire>,
//     KzgrsSamplingBackend<ChaCha20Rng>,
//     SamplingAdapter,
//     ChaCha20Rng,
//     SamplingStorageAdapter<DaShare, Wire>,
//     KzgrsDaVerifier,
//     VerifierNetworkAdapter<NomosDaMembership>,
//     VerifierStorageAdapter<DaShare, Wire>,
//     SystemTimeBackend,
//     HttApiAdapter<NomosDaMembership>,
// >;

// pub type NodeCryptarchia =
//     Cryptarchia<nomos_da_sampling::network::adapters::validator::Libp2pAdapter<NomosDaMembership>>;

// pub type TxMempool = TxMempoolService<
//     MempoolNetworkAdapter<Tx, <Tx as Transaction>::Hash>,
//     MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
// >;

// pub type DaMempool = DaMempoolService<
//     MempoolNetworkAdapter<BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
//     MockPool<HeaderId, BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
//     KzgrsSamplingBackend<ChaCha20Rng>,
//     nomos_da_sampling::network::adapters::validator::Libp2pAdapter<NomosDaMembership>,
//     ChaCha20Rng,
//     SamplingStorageAdapter<DaShare, Wire>,
//     KzgrsDaVerifier,
//     VerifierNetworkAdapter<NomosDaMembership>,
//     VerifierStorageAdapter<DaShare, Wire>,
//     HttApiAdapter<NomosDaMembership>,
// >;

// pub type DaIndexer<SamplingAdapter> = DataIndexerService<
//     // Indexer specific.
//     DaShare,
//     IndexerStorageAdapter<Wire, BlobInfo>,
//     CryptarchiaConsensusAdapter<Tx, BlobInfo>,
//     // Cryptarchia specific, should be the same as in `Cryptarchia` type
// above.
//     cryptarchia_consensus::network::adapters::libp2p::LibP2pAdapter<Tx,
// BlobInfo>,     cryptarchia_consensus::blend::adapters::libp2p::LibP2pAdapter<
//         BlendNetworkAdapter,
//         Tx,
//         BlobInfo,
//     >,
//     MockPool<HeaderId, Tx, <Tx as Transaction>::Hash>,
//     MempoolNetworkAdapter<Tx, <Tx as Transaction>::Hash>,
//     MockPool<HeaderId, BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
//     MempoolNetworkAdapter<BlobInfo, <BlobInfo as DispersedBlobInfo>::BlobId>,
//     FillSizeWithTx<MB16, Tx>,
//     FillSizeWithBlobs<MB16, BlobInfo>,
//     RocksBackend<Wire>,
//     KzgrsSamplingBackend<ChaCha20Rng>,
//     SamplingAdapter,
//     ChaCha20Rng,
//     SamplingStorageAdapter<DaShare, Wire>,
//     KzgrsDaVerifier,
//     VerifierNetworkAdapter<NomosDaMembership>,
//     VerifierStorageAdapter<DaShare, Wire>,
//     SystemTimeBackend,
//     HttApiAdapter<NomosDaMembership>,
// >;

// pub type NodeDaIndexer =
//     DaIndexer<nomos_da_sampling::network::adapters::validator::Libp2pAdapter<NomosDaMembership>>;

// pub type DaSampling<SamplingAdapter> = DaSamplingService<
//     KzgrsSamplingBackend<ChaCha20Rng>,
//     SamplingAdapter,
//     ChaCha20Rng,
//     SamplingStorageAdapter<DaShare, Wire>,
//     KzgrsDaVerifier,
//     VerifierNetworkAdapter<NomosDaMembership>,
//     VerifierStorageAdapter<DaShare, Wire>,
//     HttApiAdapter<NomosDaMembership>,
// >;

// pub type NodeDaSampling =
// DaSampling<SamplingLibp2pAdapter<NomosDaMembership>>;

// pub type DaVerifier<VerifierAdapter> =
//     DaVerifierService<KzgrsDaVerifier, VerifierAdapter,
// VerifierStorageAdapter<DaShare, Wire>>;

// pub type NodeDaVerifier =
// DaVerifier<VerifierNetworkAdapter<FillFromNodeList>>;

pub type NomosTimeService = TimeService<SystemTimeBackend, RuntimeServiceId>;

#[derive_services]
pub struct Nomos {
    #[cfg(feature = "tracing")]
    tracing: Tracing<RuntimeServiceId>,
    network: NetworkService<NetworkBackend, RuntimeServiceId>,
    blend: BlendService<BlendBackend, BlendNetworkAdapter<RuntimeServiceId>, RuntimeServiceId>,
    // da_indexer: OpaqueServiceHandle<NodeDaIndexer>,
    // da_verifier: OpaqueServiceHandle<NodeDaVerifier>,
    // da_sampling: OpaqueServiceHandle<NodeDaSampling>,
    da_network: DaNetworkService<DaNetworkValidatorBackend<NomosDaMembership>, RuntimeServiceId>,
    // cl_mempool: OpaqueServiceHandle<TxMempool>,
    // da_mempool: OpaqueServiceHandle<DaMempool>,
    // cryptarchia: OpaqueServiceHandle<NodeCryptarchia>,
    time: NomosTimeService,
    // http: OpaqueServiceHandle<NomosApiService>,
    storage: StorageService<RocksBackend<Wire>, RuntimeServiceId>,
    system_sig: SystemSig<RuntimeServiceId>,
}

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
