use nomos_blend_scheduling::{
    membership::{Membership, Node},
    message_blend::CryptographicProcessorSettings,
};
use serde::{Deserialize, Serialize};

use crate::core::settings::TimingSettings;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlendConfig<BackendSettings, NodeId> {
    pub backend: BackendSettings,
    pub crypto: CryptographicProcessorSettings,
    pub time: TimingSettings,
    pub membership: Vec<Node<NodeId>>,
}

impl<BackendSettings, NodeId> BlendConfig<BackendSettings, NodeId>
where
    NodeId: Clone,
{
    pub fn membership(&self) -> Membership<NodeId> {
        Membership::new(&self.membership, None)
    }
}
