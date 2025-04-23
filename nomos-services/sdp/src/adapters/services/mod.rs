use nomos_sdp_core::ledger;

pub trait SdpServicesAdapter: ledger::ActivityContract {
    fn new() -> Self;
}
