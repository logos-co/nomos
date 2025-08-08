use std::collections::HashMap;

use libp2p::{Multiaddr, PeerId};
use nomos_da_network_core::addressbook::AddressBookHandler;

pub type AddressBookSnapshot<Id> = HashMap<Id, Multiaddr>;

#[derive(Debug, Clone, Default)]
pub struct AddressBook {
    peers: HashMap<PeerId, Multiaddr>,
}

pub trait AddressBookMut: AddressBookHandler {
    fn update(&mut self, new_peers: AddressBookSnapshot<Self::Id>);
}

impl AddressBookHandler for AddressBook {
    type Id = PeerId;

    fn get_address(&self, peer_id: &Self::Id) -> Option<Multiaddr> {
        self.peers.get(peer_id).cloned()
    }
}

impl AddressBookMut for AddressBook {
    fn update(&mut self, new_peers: AddressBookSnapshot<Self::Id>) {
        self.peers.extend(new_peers);
    }
}
