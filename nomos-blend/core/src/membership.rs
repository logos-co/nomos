use std::{collections::HashSet, hash::Hash};

use multiaddr::Multiaddr;
use nomos_blend_message::BlendMessage;
use rand::{
    seq::{IteratorRandom as _, SliceRandom as _},
    Rng,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct Membership<NodeId, M>
where
    M: BlendMessage,
{
    remote_nodes: Vec<Node<NodeId, M::PublicKey>>,
    local_node: Node<NodeId, M::PublicKey>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node<Id, K> {
    /// An unique identifier of the node,
    /// which is usually corresponding to the network node identifier
    /// but depending on the network backend.
    pub id: Id,
    /// A listening address
    pub address: Multiaddr,
    /// A public key used for the blend message encryption
    pub public_key: K,
}

impl<NodeId, M> Membership<NodeId, M>
where
    NodeId: Hash + Eq,
    M: BlendMessage,
    M::PublicKey: PartialEq,
{
    pub fn new(nodes: Vec<Node<NodeId, M::PublicKey>>, local_public_key: &M::PublicKey) -> Self {
        let mut remote_nodes = Vec::with_capacity(nodes.len() - 1);
        let mut local_node = None;
        for node in nodes {
            if node.public_key == *local_public_key {
                local_node = Some(node);
            } else {
                remote_nodes.push(node);
            }
        }

        Self {
            remote_nodes,
            local_node: local_node.expect("Local node not found"),
        }
    }

    pub fn choose_remote_nodes<R: Rng>(
        &self,
        rng: &mut R,
        amount: usize,
    ) -> Vec<&Node<NodeId, M::PublicKey>> {
        self.remote_nodes.choose_multiple(rng, amount).collect()
    }

    pub fn filter_and_choose_remote_nodes<R: Rng>(
        &self,
        rng: &mut R,
        amount: usize,
        exclude_peers: &HashSet<NodeId>,
    ) -> Vec<&Node<NodeId, M::PublicKey>> {
        self.remote_nodes
            .iter()
            .filter(|node| !exclude_peers.contains(&node.id))
            .choose_multiple(rng, amount)
    }

    pub const fn local_node(&self) -> &Node<NodeId, M::PublicKey> {
        &self.local_node
    }

    pub const fn size(&self) -> usize {
        self.remote_nodes.len() + 1
    }
}
