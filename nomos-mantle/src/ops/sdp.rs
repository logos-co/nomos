pub type SDPDeclareOp = nomos_sdp_core::DeclarationMessage;
pub type SDPWithdrawOp = nomos_sdp_core::WithdrawMessage;
// TODO: Abstract metadata
pub type SDPActivityOp = nomos_sdp_core::ActiveMessage<Vec<u8>>;
