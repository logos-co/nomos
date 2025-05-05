use nomos_sdp_core::ledger::RewardsRequestSender;

pub mod rewards_sender;

pub trait SdpRewardsAdapter: RewardsRequestSender {
    fn new() -> Self;
}
