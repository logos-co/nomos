pub mod api;
pub mod config;
pub mod generic_services;

use core::convert::Infallible;

use async_trait::async_trait;
use color_eyre::eyre::Result;
use generic_services::VerifierMempoolAdapter;
use kzgrs_backend::common::share::DaShare;
pub use kzgrs_backend::dispersal::BlobInfo;
use nomos_blend_message::{
    crypto::{
        keys::Ed25519PrivateKey,
        proofs::{
            quota::{inputs::prove::PublicInputs, ProofOfQuota},
            selection::{inputs::VerifyInputs, ProofOfSelection},
        },
    },
    encap::ProofsVerifier,
};
use nomos_blend_scheduling::message_blend::{BlendLayerProof, ProofsGenerator, SessionInfo};
pub use nomos_blend_service::{
    core::{
        backends::libp2p::Libp2pBlendBackend as BlendBackend,
        network::libp2p::Libp2pAdapter as BlendNetworkAdapter,
    },
    membership::service::Adapter as BlendMembershipAdapter,
};
pub use nomos_core::{
    codec,
    header::HeaderId,
    mantle::{select::FillSize as FillSizeWithTx, Transaction},
};
use nomos_core::{crypto::ZkHash, mantle::SignedMantleTx};
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
pub use nomos_mempool::network::adapters::libp2p::{
    Libp2pAdapter as MempoolNetworkAdapter, Settings as MempoolAdapterSettings,
};
pub use nomos_network::backends::libp2p::Libp2p as NetworkBackend;
pub use nomos_storage::backends::{
    rocksdb::{RocksBackend, RocksBackendSettings},
    SerdeOp,
};
pub use nomos_system_sig::SystemSig;
use nomos_time::backends::NtpTimeBackend;
#[cfg(feature = "tracing")]
pub use nomos_tracing_service::Tracing;
use overwatch::derive_services;
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

/// Membership used by the DA Network service.
pub type NomosDaMembership = HistoryAware<PeerId>;
type DaMembershipStorage = DaMembershipStorageGeneric<RuntimeServiceId>;
pub type DaNetworkApiAdapter = HttApiAdapter<DaMembershipHandler<NomosDaMembership>, DaAddressbook>;

#[cfg(feature = "tracing")]
pub(crate) type TracingService = Tracing<RuntimeServiceId>;

pub(crate) type NetworkService = nomos_network::NetworkService<NetworkBackend, RuntimeServiceId>;

// TODO: Replace this with the actual verifier once the verification inputs are
// successfully fetched by the Blend service.
#[derive(Clone)]
pub struct MockProofsVerifier;

impl ProofsVerifier for MockProofsVerifier {
    type Error = Infallible;

    fn new() -> Self {
        Self
    }

    fn verify_proof_of_quota(
        &self,
        _proof: ProofOfQuota,
        _inputs: &PublicInputs,
    ) -> Result<ZkHash, Self::Error> {
        use groth16::Field as _;

        Ok(ZkHash::ZERO)
    }

    fn verify_proof_of_selection(
        &self,
        _proof: ProofOfSelection,
        _inputs: &VerifyInputs,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub struct MockProofsGenerator;

#[async_trait]
impl ProofsGenerator for MockProofsGenerator {
    fn new(_session_info: SessionInfo) -> Self {
        Self
    }

    /// Get or generate the next core `PoQ`, if the maximum allowance has not
    /// been reached.
    async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof> {
        Some(BlendLayerProof {
            ephemeral_signing_key: Ed25519PrivateKey::generate(),
            proof_of_quota: ProofOfQuota::from_bytes_unchecked([0; _]),
            proof_of_selection: ProofOfSelection::from_bytes_unchecked([0; _]),
        })
    }
    /// Get or generate the next leadership `PoQ`, if the maximum allowance has
    /// not been reached.
    async fn get_next_leadership_proof(&mut self) -> Option<BlendLayerProof> {
        Some(BlendLayerProof {
            ephemeral_signing_key: Ed25519PrivateKey::generate(),
            proof_of_quota: ProofOfQuota::from_bytes_unchecked([0; _]),
            proof_of_selection: ProofOfSelection::from_bytes_unchecked([0; _]),
        })
    }
}

pub type BlendProofsGenerator = MockProofsGenerator;
pub type BlendProofsVerifier = MockProofsVerifier;

pub(crate) type BlendCoreService = nomos_blend_service::core::BlendService<
    BlendBackend,
    PeerId,
    BlendNetworkAdapter<RuntimeServiceId>,
    BlendMembershipAdapter<MembershipService<RuntimeServiceId>, PeerId>,
    BlendProofsGenerator,
    BlendProofsVerifier,
    RuntimeServiceId,
>;

pub(crate) type BlendEdgeService = nomos_blend_service::edge::BlendService<
    nomos_blend_service::edge::backends::libp2p::Libp2pBlendBackend,
    PeerId,
    <BlendNetworkAdapter<RuntimeServiceId> as nomos_blend_service::core::network::NetworkAdapter<
        RuntimeServiceId,
    >>::BroadcastSettings,
    BlendMembershipAdapter<MembershipService<RuntimeServiceId>, PeerId>,
    BlendProofsGenerator,
    RuntimeServiceId,
>;

pub(crate) type BlendService =
    nomos_blend_service::BlendService<BlendCoreService, BlendEdgeService, RuntimeServiceId>;

pub(crate) type BlockBroadcastService = broadcast_service::BlockBroadcastService<RuntimeServiceId>;

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

pub(crate) type WalletService =
    nomos_wallet::WalletService<CryptarchiaService, RocksBackend, RuntimeServiceId>;

pub(crate) type ApiStorageAdapter<RuntimeServiceId> =
    nomos_api::http::storage::adapters::rocksdb::RocksAdapter<RuntimeServiceId>;

pub(crate) type ApiService = nomos_api::ApiService<
    AxumBackend<
        DaShare,
        NomosDaMembership,
        DaMembershipAdapter<RuntimeServiceId>,
        DaMembershipStorage,
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
        KzgrsSamplingBackend,
        nomos_da_sampling::network::adapters::validator::Libp2pAdapter<
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
        MB16,
    >,
    RuntimeServiceId,
>;

type StorageService = nomos_storage::StorageService<RocksBackend, RuntimeServiceId>;

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
    cl_mempool: ClMempoolService,
    cryptarchia: CryptarchiaService,
    block_broadcast: BlockBroadcastService,
    membership: MembershipService<RuntimeServiceId>,
    sdp: SdpService<RuntimeServiceId>,
    time: TimeService,
    http: ApiService,
    storage: StorageService,
    system_sig: SystemSigService,
    wallet: WalletService,
    #[cfg(feature = "testing")]
    testing_http: TestingApiService<RuntimeServiceId>,
}
