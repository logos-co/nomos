use nomos_sdp_core::ledger;

pub trait SdpDeclarationAdapter: ledger::DeclarationsRepository {
    fn new() -> Self;
}
