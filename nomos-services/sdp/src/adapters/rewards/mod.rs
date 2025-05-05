use nomos_sdp_core::ledger;

pub mod rewards_sender;

pub trait SdpRewardsAdapter: ledger::ActivityContract {
    fn new() -> Self;
}
