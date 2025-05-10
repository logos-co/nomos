use nomos_sdp_core::ledger;

pub mod verifier;

pub trait SdpStakesVerifierAdapter: ledger::StakesVerifier {
    fn new() -> Self;
}
