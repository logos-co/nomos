use core::convert::Infallible;

use async_trait::async_trait;
use nomos_blend_message::crypto::{
    keys::Ed25519PrivateKey,
    proofs::{
        quota::{inputs::prove::PublicInputs, ProofOfQuota},
        selection::{inputs::VerifyInputs, ProofOfSelection},
    },
};
use nomos_blend_scheduling::message_blend::{BlendLayerProof, SessionInfo};
use nomos_blend_service::{membership::service::Adapter, ProofsGenerator, ProofsVerifier};
use nomos_core::crypto::ZkHash;
use nomos_libp2p::PeerId;

use crate::generic_services::MembershipService;

// TODO: Replace this with the actual verifier once the verification inputs are
// successfully fetched by the Blend service.
#[derive(Clone)]
pub struct BlendProofsVerifier;

impl ProofsVerifier for BlendProofsVerifier {
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

pub struct BlendProofsGenerator;

#[async_trait]
impl ProofsGenerator for BlendProofsGenerator {
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

pub type BlendMembershipAdapter<RuntimeServiceId> =
    Adapter<MembershipService<RuntimeServiceId>, PeerId>;
pub type BlendCoreService<RuntimeServiceId> = nomos_blend_service::core::BlendService<
    nomos_blend_service::core::backends::libp2p::Libp2pBlendBackend,
    PeerId,
    nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
    BlendMembershipAdapter<RuntimeServiceId>,
    BlendProofsGenerator,
    BlendProofsVerifier,
    RuntimeServiceId,
>;
pub type BlendEdgeService<RuntimeServiceId> = nomos_blend_service::edge::BlendService<
        nomos_blend_service::edge::backends::libp2p::Libp2pBlendBackend,
        PeerId,
        <nomos_blend_service::core::network::libp2p::Libp2pAdapter<RuntimeServiceId> as nomos_blend_service::core::network::NetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
        BlendMembershipAdapter<RuntimeServiceId>,
        BlendProofsGenerator,
        RuntimeServiceId
    >;
pub type BlendService<RuntimeServiceId> = nomos_blend_service::BlendService<
    BlendCoreService<RuntimeServiceId>,
    BlendEdgeService<RuntimeServiceId>,
    RuntimeServiceId,
>;
