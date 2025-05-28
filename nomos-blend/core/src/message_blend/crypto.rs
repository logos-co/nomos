use std::hash::Hash;

use nomos_blend_message::BlendMessage;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::membership::Membership;

/// [`CryptographicProcessor`] is responsible for wrapping and unwrapping
/// messages for the message indistinguishability.
pub struct CryptographicProcessor<NodeId, R, M>
where
    M: BlendMessage,
{
    settings: CryptographicProcessorSettings<M::PrivateKey>,
    membership: Membership<NodeId, M>,
    rng: R,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CryptographicProcessorSettings<K> {
    pub private_key: K,
    pub num_blend_layers: usize,
}

impl<NodeId, R, M> CryptographicProcessor<NodeId, R, M>
where
    NodeId: Hash + Eq,
    R: RngCore,
    M: BlendMessage,
    M::PublicKey: Clone + PartialEq,
{
    pub const fn new(
        settings: CryptographicProcessorSettings<M::PrivateKey>,
        membership: Membership<NodeId, M>,
        rng: R,
    ) -> Self {
        Self {
            settings,
            membership,
            rng,
        }
    }

    pub fn wrap_message(&mut self, message: &[u8]) -> Result<Vec<u8>, M::Error> {
        // TODO: Use the actual Sphinx encoding instead of mock.
        let public_keys = self
            .membership
            .choose_remote_nodes(&mut self.rng, self.settings.num_blend_layers)
            .iter()
            .map(|node| node.public_key.clone())
            .collect::<Vec<_>>();

        M::build(message, &public_keys)
    }

    pub fn unwrap_message(&self, message: &[u8]) -> Result<(Vec<u8>, bool), M::Error> {
        M::unwrap(message, &self.settings.private_key)
    }
}
