use std::hash::Hash;

use libp2p::Multiaddr;
use nomos_blend_message::crypto::keys::{Ed25519PrivateKey, Ed25519PublicKey};
use nomos_blend_scheduling::{
    membership::{Membership, Node},
    message_blend::SessionInfo,
};

use crate::mock_poq_inputs;

pub fn membership<NodeId>(ids: &[NodeId], local_id: NodeId) -> Membership<NodeId>
where
    NodeId: Clone + Eq + Hash,
    [u8; 32]: From<NodeId>,
{
    Membership::new(
        &ids.iter()
            .map(|id| Node {
                id: id.clone(),
                address: Multiaddr::empty(),
                public_key: key(id.clone()).1,
            })
            .collect::<Vec<_>>(),
        &key(local_id).1,
    )
}

pub fn mock_session_info() -> SessionInfo {
    let (public_inputs, private_inputs) = mock_poq_inputs();
    SessionInfo {
        public_inputs,
        private_inputs,
        local_node_index: None,
        membership_size: 0,
    }
}

pub fn key<NodeId>(id: NodeId) -> (Ed25519PrivateKey, Ed25519PublicKey)
where
    [u8; 32]: From<NodeId>,
{
    let private_key = Ed25519PrivateKey::from(<[u8; 32]>::from(id));
    let public_key = private_key.public_key();
    (private_key, public_key)
}
