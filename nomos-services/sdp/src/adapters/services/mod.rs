use nomos_sdp_core::ledger;

pub mod services_repository;

pub trait SdpServicesAdapter: ledger::ServicesRepository {
    fn new() -> Self;
}
