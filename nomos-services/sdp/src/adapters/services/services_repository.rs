use async_trait::async_trait;
use nomos_sdp_core::ledger;

#[derive(Debug, Clone)]
pub struct LedgerServicesAdapter;

pub trait SdpServicesAdapter: ledger::ServicesRepository {
    fn new() -> Self;
}
impl SdpServicesAdapter for LedgerServicesAdapter {
    fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl ledger::ServicesRepository for LedgerServicesAdapter {
    async fn get_parameters(
        &self,
        _service_type: nomos_sdp_core::ServiceType,
    ) -> Result<nomos_sdp_core::ServiceParameters, ledger::ServicesRepositoryError> {
        todo!()
    }
}
