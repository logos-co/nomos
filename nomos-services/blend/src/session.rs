use nomos_blend_message::crypto::proofs::quota::inputs::prove::{
    private::ProofOfCoreQuotaInputs, public::CoreInputs,
};
use nomos_blend_scheduling::membership::Membership;

#[derive(Clone)]
/// All info that Blend services need to be available on new sessions.
pub struct CoreSessionInfo<NodeId> {
    pub public: CoreSessionPublicInfo<NodeId>,
    pub private: ProofOfCoreQuotaInputs,
}

#[derive(Clone)]
/// All info that Blend services need to be available on new sessions.
pub struct CoreSessionPublicInfo<NodeId> {
    /// The list of core Blend nodes for the new session.
    pub membership: Membership<NodeId>,
    /// The session number.
    pub session: u64,
    /// The set of public inputs to verify core `PoQ`s.
    pub poq_core_public_inputs: CoreInputs,
}
