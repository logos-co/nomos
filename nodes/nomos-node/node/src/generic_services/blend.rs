use core::convert::Infallible;

use async_trait::async_trait;
use nomos_blend_message::crypto::{
    keys::Ed25519PrivateKey,
    proofs::{
        quota::{inputs::prove::PublicInputs, ProofOfQuota},
        selection::{inputs::VerifyInputs, ProofOfSelection},
    },
    random_sized_bytes,
};
use nomos_blend_scheduling::message_blend::{BlendLayerProof, SessionInfo};
use nomos_blend_service::{membership::service::Adapter, ProofsGenerator, ProofsVerifier};
use nomos_core::{codec::SerdeOp as _, crypto::ZkHash};
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

pub struct BlendProofsGenerator {
    membership_size: usize,
    local_node_index: Option<usize>,
}

#[async_trait]
impl ProofsGenerator for BlendProofsGenerator {
    fn new(
        SessionInfo {
            membership_size,
            local_node_index,
            ..
        }: SessionInfo,
    ) -> Self {
        Self {
            membership_size,
            local_node_index,
        }
    }

    async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof> {
        Some(loop_until_valid_proof(
            self.membership_size,
            self.local_node_index,
        ))
    }

    async fn get_next_leadership_proof(&mut self) -> Option<BlendLayerProof> {
        Some(loop_until_valid_proof(
            self.membership_size,
            self.local_node_index,
        ))
    }
}

fn loop_until_valid_proof(
    membership_size: usize,
    local_node_index: Option<usize>,
) -> BlendLayerProof {
    // For tests, we avoid generating proofs that are addressed to the local node
    // itself, since in most tests there are only two nodes and those payload would
    // fail to be propagated.
    // This is not a precaution we need to consider in production, since there will
    // be a minimum network size that is larger than 2.
    loop {
        let Ok(proof_of_quota) =
            ProofOfQuota::deserialize(&random_sized_bytes::<{ size_of::<ProofOfQuota>() }>()[..])
        else {
            continue;
        };
        let Ok(proof_of_selection) = ProofOfSelection::deserialize::<ProofOfSelection>(
            &random_sized_bytes::<{ size_of::<ProofOfSelection>() }>()[..],
        ) else {
            continue;
        };
        let Ok(expected_index) = proof_of_selection.expected_index(membership_size) else {
            continue;
        };
        if Some(expected_index) != local_node_index {
            return BlendLayerProof {
                ephemeral_signing_key: Ed25519PrivateKey::generate(),
                proof_of_quota,
                proof_of_selection,
            };
        }
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
