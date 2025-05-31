use async_trait::async_trait;
use nomos_sdp_core::ledger::{self, ServicesRepository};

pub trait SdpServicesAdapter: ServicesRepository {
    fn new() -> Self;
}

#[derive(Debug, Clone)]
pub struct LedgerServicesAdapter;

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
