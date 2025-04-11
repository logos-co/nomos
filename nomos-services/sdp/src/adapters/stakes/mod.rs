use nomos_sdp_core::ledger;

pub trait SdpStakesVerifierAdapter: ledger::StakesVerifier {
    fn new() -> Self;
}
