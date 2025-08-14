use nomos_blend_scheduling::{
    membership::Membership, message_blend::CryptographicProcessorSettings,
};
use serde::{Deserialize, Serialize};

use crate::settings::{MembershipSettings, TimingSettings};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlendConfig<BackendSettings, NodeId> {
    pub backend: BackendSettings,
    pub crypto: CryptographicProcessorSettings,
    pub time: TimingSettings,
    pub membership: MembershipSettings<NodeId>,
}

impl<BackendSettings, NodeId> BlendConfig<BackendSettings, NodeId>
where
    NodeId: Clone,
{
    pub fn membership(&self) -> Membership<NodeId> {
        self.membership.membership(None)
    }
}
