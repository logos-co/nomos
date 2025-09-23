use std::hash::Hash;

use libp2p::Multiaddr;
use nomos_blend_message::crypto::keys::{Ed25519PrivateKey, Ed25519PublicKey};
use nomos_blend_scheduling::{
    membership::{Membership, Node},
    message_blend::{PrivateInputs, SessionInfo},
};

use crate::{mock_poq_inputs, test_utils::epoch::mock_pol_epoch};

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
    let pol_epoch = mock_pol_epoch();
    SessionInfo {
        public_inputs,
        private_inputs: PrivateInputs {
            aged_path: pol_epoch.poq_private_inputs.aged_path,
            aged_selector: pol_epoch.poq_private_inputs.aged_selector,
            core_path: private_inputs.core_path,
            core_path_selectors: private_inputs.core_path_selectors,
            core_sk: private_inputs.core_sk,
            note_value: pol_epoch.poq_private_inputs.note_value,
            output_number: pol_epoch.poq_private_inputs.output_number,
            pol_secret_key: pol_epoch.poq_private_inputs.pol_secret_key,
            slot: pol_epoch.poq_private_inputs.slot,
            slot_secret: pol_epoch.poq_private_inputs.slot_secret,
            slot_secret_path: pol_epoch.poq_private_inputs.slot_secret_path,
            starting_slot: pol_epoch.poq_private_inputs.starting_slot,
            transaction_hash: pol_epoch.poq_private_inputs.transaction_hash,
        },
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
