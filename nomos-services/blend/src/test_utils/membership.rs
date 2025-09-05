use libp2p::Multiaddr;
use nomos_blend_message::crypto::{Ed25519PrivateKey, Ed25519PublicKey};
use nomos_blend_scheduling::membership::{Membership, Node};

pub type NodeId = u8;

pub fn membership(ids: &[NodeId], local_id: NodeId) -> Membership<NodeId> {
    Membership::new(
        &ids.iter()
            .map(|id| Node {
                id: *id,
                address: Multiaddr::empty(),
                public_key: key(*id).1,
            })
            .collect::<Vec<_>>(),
        &key(local_id).1,
    )
}

pub fn key(id: NodeId) -> (Ed25519PrivateKey, Ed25519PublicKey) {
    let private_key = Ed25519PrivateKey::from([id; 32]);
    let public_key = private_key.public_key();
    (private_key, public_key)
}
