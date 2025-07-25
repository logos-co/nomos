use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use libp2p::{Multiaddr, PeerId};
use nomos_da_network_core::addressbook::{AddressBookHandler, AddressBookMut};

#[derive(Default, Debug, Clone)]
pub struct MockAddressBook {
    peers: Arc<Mutex<HashMap<PeerId, Multiaddr>>>,
}

impl AddressBookHandler for MockAddressBook {
    type Id = PeerId;

    fn get_address(&self, peer_id: &Self::Id) -> Option<Multiaddr> {
        let peers = self.peers.lock().unwrap();
        peers.get(peer_id).cloned()
    }
}

impl AddressBookMut for MockAddressBook {
    fn update(&self, new_peers: HashMap<Self::Id, Multiaddr>) {
        let mut peers = self.peers.lock().unwrap();
        peers.extend(new_peers);
    }
}
