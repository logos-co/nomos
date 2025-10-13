use nomos_blend_message::crypto::proofs::quota::inputs::prove::{
    private::ProofOfLeadershipQuotaInputs, public::CoreInputs,
};
use nomos_blend_scheduling::membership::Membership;
use nomos_core::crypto::ZkHash;

/// All info that Blend services need to be available on new sessions.
pub struct CoreSessionPublicInfo<NodeId> {
    /// The list of core Blend nodes for the new session.
    pub membership: Membership<NodeId>,
    /// The session number.
    pub session: u64,
    /// The set of public inputs to verify core `PoQ`s.
    pub poq_core_inputs: CoreInputs,
}
