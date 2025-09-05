use core::num::NonZeroU64;
use std::hash::Hash;

use nomos_blend_scheduling::{
    membership::{Membership, Node},
    message_blend::CryptographicProcessorSettings,
};
use serde::{Deserialize, Serialize};

use crate::settings::TimingSettings;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlendConfig<BackendSettings, NodeId> {
    pub backend: BackendSettings,
    pub crypto: CryptographicProcessorSettings,
    pub time: TimingSettings,
    // TODO: Remove this and use the membership service stream instead: https://github.com/logos-co/nomos/issues/1532
    pub membership: Vec<Node<NodeId>>,
    pub minimum_network_size: NonZeroU64,
}

impl<BackendSettings, NodeId> BlendConfig<BackendSettings, NodeId>
where
    NodeId: Clone + Eq + Hash,
{
    pub fn membership(&self) -> Membership<NodeId> {
        let local_signing_pubkey = self.crypto.signing_private_key.public_key();
        Membership::new(&self.membership, &local_signing_pubkey)
    }
}
