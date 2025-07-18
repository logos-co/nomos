use std::{collections::HashSet, hash::Hash};

use multiaddr::Multiaddr;
use nomos_blend_message::crypto::Ed25519PublicKey;
use rand::{
    seq::{IteratorRandom as _, SliceRandom as _},
    Rng,
};
use serde::{Deserialize, Serialize};

use crate::serde::ed25519_pubkey_hex;

#[derive(Clone, Debug)]
pub struct Membership<NodeId> {
    remote_nodes: Vec<Node<NodeId>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node<Id> {
    /// An unique identifier of the node,
    /// which is usually corresponding to the network node identifier
    /// but depending on the network backend.
    pub id: Id,
    /// A listening address
    pub address: Multiaddr,
    /// A public key used for the blend message encryption
    #[serde(with = "ed25519_pubkey_hex")]
    pub public_key: Ed25519PublicKey,
}

impl<NodeId> Membership<NodeId>
where
    NodeId: Clone,
{
    #[must_use]
    pub fn new(nodes: &[Node<NodeId>], local_public_key: Option<&Ed25519PublicKey>) -> Self {
        Self {
            remote_nodes: nodes
                .iter()
                .filter(|node| !matches!(local_public_key, Some(key) if node.public_key == *key))
                .cloned()
                .collect(),
        }
    }
}

impl<NodeId> Membership<NodeId> {
    pub fn choose_remote_nodes<R: Rng>(
        &self,
        rng: &mut R,
        amount: usize,
    ) -> impl Iterator<Item = &Node<NodeId>> {
        self.remote_nodes.choose_multiple(rng, amount)
    }

    #[must_use]
    pub const fn size(&self) -> usize {
        self.remote_nodes.len() + 1
    }
}

impl<NodeId> Membership<NodeId>
where
    NodeId: Eq + Hash,
{
    pub fn filter_and_choose_remote_nodes<R: Rng>(
        &self,
        rng: &mut R,
        amount: usize,
        exclude_peers: &HashSet<NodeId>,
    ) -> Vec<&Node<NodeId>> {
        self.remote_nodes
            .iter()
            .filter(|node| !exclude_peers.contains(&node.id))
            .choose_multiple(rng, amount)
    }

    // TODO: Change internal structure to a hashset/hashmap for O(1) lookup.
    pub fn contains_remote(&self, node_id: &NodeId) -> bool {
        self.remote_nodes.iter().any(|n| n.id == *node_id)
    }
}
