use std::{collections::HashMap, sync::Arc};

use arc_swap::ArcSwap;
use libp2p::{Multiaddr, PeerId};
use nomos_da_network_core::addressbook::AddressBookHandler;

pub type AddressBookSnapshot<Id> = HashMap<Id, Multiaddr>;

pub trait AddressBookMut: AddressBookHandler {
    fn update(&self, new_peers: AddressBookSnapshot<Self::Id>);
}

#[derive(Debug, Clone)]
pub struct AddressBook {
    peers: Arc<ArcSwap<AddressBookSnapshot<PeerId>>>,
}

impl Default for AddressBook {
    fn default() -> Self {
        Self {
            peers: Arc::new(ArcSwap::from_pointee(AddressBookSnapshot::new())),
        }
    }
}

impl AddressBookHandler for AddressBook {
    type Id = PeerId;

    fn get_address(&self, peer_id: &Self::Id) -> Option<Multiaddr> {
        let peers = self.peers.load();
        peers.get(peer_id).cloned()
    }
}

impl AddressBookMut for AddressBook {
    fn update(&self, new_peers: AddressBookSnapshot<Self::Id>) {
        self.peers.store(Arc::new(new_peers));
    }
}

// Implementations for Arc<T> to allow transparent usage
impl<T> AddressBookMut for Arc<T>
where
    T: AddressBookMut,
{
    fn update(&self, new_peers: AddressBookSnapshot<Self::Id>) {
        (**self).update(new_peers);
    }
}
