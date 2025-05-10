use nomos_sdp_core::ledger;

pub mod repository;

pub trait SdpDeclarationAdapter: ledger::DeclarationsRepository {
    fn new() -> Self;
}
