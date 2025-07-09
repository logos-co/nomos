use std::sync::Mutex;

use libp2p::{Multiaddr, PeerId};

use crate::membership::addressbook::AddressBookMut;

struct MockAddressBook {
    addresses: Mutex<HashMap<PeerId, Multiaddr>>,
}

impl AddressBookMut for MockAddressBook {
    fn update(&self, new_peer_addresses: HashMap<PeerId, Multiaddr>) {
        let mut addresses = self.addresses.lock().unwrap();
        addresses.extend(new_peer_addresses);
    }

    fn new() -> Self {
        Self {
            addresses: Mutex::new(HashMap::new()),
        }
    }
}

impl AddressBook for MockAddressBook {
    fn get_address(&self, peer_id: &PeerId) -> Option<Multiaddr> {
        let addresses = self.addresses.lock().unwrap();
        addresses.get(peer_id).cloned()
    }
}
