use nomos_blend_scheduling::{
    membership::Membership, message_blend::SessionInfo as PoQGenerationAndVerificationInput,
};

/// All info that Blend services need to be available on new sessions.
pub struct SessionInfo<NodeId> {
    /// The list of core Blend nodes for the new session.
    pub membership: Membership<NodeId>,
    /// The set of public and private inputs required to verify Proofs of Quota
    /// in received Blend public headers.
    pub poq_generation_and_verification_inputs: PoQGenerationAndVerificationInput,
}
