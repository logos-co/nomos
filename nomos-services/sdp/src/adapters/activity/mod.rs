use nomos_sdp_core::ledger;

pub mod contract;

pub trait SdpActivityAdapter: ledger::ActivityContract {
    fn new() -> Self;
}
