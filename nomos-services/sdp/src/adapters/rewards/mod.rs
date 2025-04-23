use nomos_sdp_core::ledger;

pub trait SdpRewardsAdapter: ledger::ActivityContract {
    fn new() -> Self;
}
