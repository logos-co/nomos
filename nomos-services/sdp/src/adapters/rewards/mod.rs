use nomos_sdp_core::ledger;

pub trait SdpRewardsAdapter: ledger::RewardsRequestSender {
    fn new() -> Self;
}
