pub mod mock;

use libp2p::{Multiaddr, PeerId};

pub trait AddressBook {
    fn get_address(&self, peer_id: &PeerId) -> Option<Multiaddr>;
}

pub trait AddressBookMut: AddressBook {
    fn new() -> Self;
    fn update(&self, new_peer_addresses: HashMap<PeerId, Multiaddr>);
}
