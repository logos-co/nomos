use nomos_sdp::backends::FinalizedBlockEvent;
use nomos_sdp_core::ServiceType;

use super::{MembershipBackend, MembershipBackendSettings};

pub struct DequeMembershipBackend {
    _settings: MembershipBackendSettings,
}

#[async_trait::async_trait]
impl MembershipBackend for DequeMembershipBackend {
    fn init(settings: MembershipBackendSettings) -> Self {
        Self {
            _settings: settings,
        }
    }

    async fn get_snapshot_at(
        &self,
        _service_type: ServiceType,
        _index: i32,
    ) -> Result<(), overwatch::DynError> {
        todo!()
    }

    async fn update(&self, _update: FinalizedBlockEvent) -> Result<(), overwatch::DynError> {
        todo!()
    }
}
