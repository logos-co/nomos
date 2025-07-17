use async_trait::async_trait;
use nomos_core::sdp;

use crate::backends::{ServicesRepository, ServicesRepositoryError};

#[derive(Debug, Clone)]
pub struct LedgerServicesAdapter;

pub trait SdpServicesAdapter: ServicesRepository {
    fn new() -> Self;
}
impl SdpServicesAdapter for LedgerServicesAdapter {
    fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl ServicesRepository for LedgerServicesAdapter {
    async fn get_parameters(
        &self,
        _service_type: sdp::ServiceType,
    ) -> Result<sdp::ServiceParameters, ServicesRepositoryError> {
        todo!()
    }
}
