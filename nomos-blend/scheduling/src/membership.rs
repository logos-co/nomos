use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use multiaddr::Multiaddr;
use nomos_blend_message::crypto::Ed25519PublicKey;
use rand::{
    seq::{IteratorRandom as _, SliceRandom as _},
    Rng,
};
use serde::{Deserialize, Serialize};

use crate::serde::ed25519_pubkey_hex;

/// A set of core nodes in a session.
#[derive(Clone, Debug)]
pub struct Membership<NodeId> {
    /// All nodes, including local and remote.
    nodes: HashMap<NodeId, Node<NodeId>>,
    /// IDs of remote nodes only, excluding the local node if present.
    /// Kept as a separate [`Vec`] to enable efficient random sampling,
    /// which is faster than sampling from the keys in [`HashMap`].
    remote_nodes: Vec<NodeId>,
    /// ID of the local node if it is part of the membership.
    local_node: Option<NodeId>,
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
    NodeId: Clone + Hash + Eq,
{
    #[must_use]
    pub fn new(nodes: &[Node<NodeId>], local_public_key: Option<&Ed25519PublicKey>) -> Self {
        let mut core_nodes = HashMap::with_capacity(nodes.len());
        let mut remote_core_nodes = Vec::with_capacity(nodes.len());
        let mut local_node = None;
        for node in nodes {
            core_nodes.insert(node.id.clone(), node.clone());
            if matches!(local_public_key, Some(key) if node.public_key == *key) {
                local_node = Some(node.id.clone());
            } else {
                remote_core_nodes.push(node.id.clone());
            }
        }

        Self {
            nodes: core_nodes,
            remote_nodes: remote_core_nodes,
            local_node,
        }
    }
}

impl<NodeId> Membership<NodeId>
where
    NodeId: Eq + Hash,
{
    /// Choose `amount` random remote nodes.
    pub fn choose_remote_nodes<R: Rng>(
        &self,
        rng: &mut R,
        amount: usize,
    ) -> impl Iterator<Item = &Node<NodeId>> {
        self.remote_nodes.choose_multiple(rng, amount).map(|id| {
            self.nodes
                .get(id)
                .expect("Node ID must exist in core nodes.")
        })
    }

    /// Choose `amount` random remote nodes excluding the given set of node IDs.
    pub fn filter_and_choose_remote_nodes<R: Rng>(
        &self,
        rng: &mut R,
        amount: usize,
        exclude_peers: &HashSet<NodeId>,
    ) -> impl Iterator<Item = &Node<NodeId>> {
        self.remote_nodes
            .iter()
            .filter(|id| !exclude_peers.contains(id))
            .choose_multiple(rng, amount)
            .into_iter()
            .map(|id| {
                self.nodes
                    .get(id)
                    .expect("Node ID must exist in core nodes.")
            })
    }

    pub fn contains(&self, node_id: &NodeId) -> bool {
        self.nodes.contains_key(node_id)
    }

    pub const fn contains_local(&self) -> bool {
        self.local_node.is_some()
    }
}

impl<NodeId> Membership<NodeId> {
    /// Returns the number of all nodes, including local and remote.
    #[must_use]
    pub fn size(&self) -> usize {
        self.nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use nomos_blend_message::crypto::Ed25519PrivateKey;
    use rand::rngs::OsRng;

    use super::*;

    #[test]
    fn test_membership_new_with_local_node() {
        let nodes = vec![node(1, 1), node(2, 2), node(3, 3)];
        let local_key = key(2);

        let membership = Membership::new(&nodes, Some(&local_key));

        assert_eq!(membership.size(), 3);
        assert_eq!(membership.remote_nodes.len(), 2);
        assert!(membership.contains_local());
        assert!(!membership.remote_nodes.contains(&2));
        assert!(membership.remote_nodes.contains(&1));
        assert!(membership.remote_nodes.contains(&3));
    }

    #[test]
    fn test_membership_new_without_local_node() {
        let nodes = vec![node(1, 1), node(2, 2), node(3, 3)];

        let membership = Membership::new(&nodes, None);

        assert_eq!(membership.size(), 3);
        assert_eq!(membership.remote_nodes.len(), 3);
        assert!(!membership.contains_local());
        assert!(membership.remote_nodes.contains(&1));
        assert!(membership.remote_nodes.contains(&2));
        assert!(membership.remote_nodes.contains(&3));
    }

    #[test]
    fn test_membership_new_empty() {
        let membership = Membership::<u32>::new(&[], None);
        assert_eq!(membership.size(), 0);
    }

    #[test]
    fn test_choose_remote_nodes() {
        let nodes = vec![node(1, 1), node(2, 2), node(3, 3), node(4, 4)];
        let membership = Membership::new(&nodes, None);

        let chosen: HashSet<_> = membership
            .choose_remote_nodes(&mut OsRng, 2)
            .map(|node| node.id)
            .collect();
        assert_eq!(chosen.len(), 2);
    }

    #[test]
    fn test_choose_remote_nodes_more_than_available() {
        let nodes = vec![node(1, 1), node(2, 2)];
        let membership = Membership::new(&nodes, None);

        let chosen: HashSet<_> = membership
            .choose_remote_nodes(&mut OsRng, 5)
            .map(|node| node.id)
            .collect();
        assert_eq!(chosen.len(), 2);
    }

    #[test]
    fn test_choose_remote_nodes_zero() {
        let nodes = vec![node(1, 1), node(2, 2)];
        let membership = Membership::new(&nodes, None);

        let mut chosen = membership.choose_remote_nodes(&mut OsRng, 0);
        assert!(chosen.next().is_none());
    }

    #[test]
    fn test_filter_and_choose_remote_nodes() {
        let nodes = vec![node(1, 1), node(2, 2), node(3, 3)];
        let membership = Membership::new(&nodes, None);
        let exclude_peers = HashSet::from([3]);

        let chosen: HashSet<_> = membership
            .filter_and_choose_remote_nodes(&mut OsRng, 2, &exclude_peers)
            .map(|node| node.id)
            .collect();
        assert_eq!(chosen.len(), 2);
    }

    #[test]
    fn test_filter_and_choose_remote_nodes_all_excluded() {
        let nodes = vec![node(1, 1), node(2, 2)];
        let membership = Membership::new(&nodes, None);
        let exclude_peers = HashSet::from([1, 2]);

        let chosen: HashSet<_> = membership
            .filter_and_choose_remote_nodes(&mut OsRng, 2, &exclude_peers)
            .map(|node| node.id)
            .collect();
        assert!(chosen.is_empty());
    }

    #[test]
    fn test_contains() {
        let nodes = vec![node(1, 1)];
        let membership = Membership::new(&nodes, None);

        assert!(membership.contains(&1));
        assert!(!membership.contains(&2));
    }

    fn key(seed: u8) -> Ed25519PublicKey {
        Ed25519PrivateKey::from([seed; 32]).public_key()
    }

    fn node(id: u32, seed: u8) -> Node<u32> {
        Node {
            id,
            address: Multiaddr::empty(),
            public_key: key(seed),
        }
    }
}
