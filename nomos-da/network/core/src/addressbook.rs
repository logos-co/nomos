use std::{collections::HashMap, fmt::Debug};

use libp2p::Multiaddr;

pub trait AddressBookHandler {
    type Id: Debug;

    fn get_address(&self, peer_id: &Self::Id) -> Option<Multiaddr>;
}

pub trait AddressBookMut: AddressBookHandler {
    fn update(&self, new_peers: HashMap<Self::Id, Multiaddr>);
}
