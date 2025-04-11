use nomos_sdp_core::ledger;

pub trait SdpServicesAdapter: ledger::RewardsRequestSender {
    fn new() -> Self;
}
