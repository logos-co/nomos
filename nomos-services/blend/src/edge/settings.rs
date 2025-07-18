use nomos_blend_scheduling::{
    membership::{Membership, Node},
    message_blend::CryptographicProcessorSettings,
};
use serde::{Deserialize, Serialize};

use crate::settings::TimingSettings;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlendEdgeSettings<BackendSettings, NodeId> {
    pub backend: BackendSettings,
    pub crypto: CryptographicProcessorSettings,
    pub time: TimingSettings,
    pub membership: Vec<Node<NodeId>>,
}

impl<BackendSettings, NodeId> BlendEdgeSettings<BackendSettings, NodeId>
where
    NodeId: Clone,
{
    pub fn membership(&self) -> Membership<NodeId> {
        Membership::new(&self.membership, None)
    }
}
