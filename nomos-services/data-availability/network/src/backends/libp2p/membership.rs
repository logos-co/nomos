use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use arc_swap::ArcSwap;
use libp2p::{Multiaddr, PeerId};
use subnetworks_assignations::MembershipHandler;

pub struct DaMembershipHandler<Membership> {
    inner: Arc<ArcSwap<Membership>>,
}

impl<Membership> DaMembershipHandler<Membership>
where
    Membership: MembershipHandler<Id = PeerId, NetworkId = u16>,
{
    pub fn new(membership: Membership) -> Self {
        Self {
            inner: Arc::new(ArcSwap::from_pointee(membership)),
        }
    }

    pub fn update(&self, members: Vec<PeerId>, addressbook: HashMap<PeerId, Multiaddr>) {
        let old_inner = self.inner.load_full();
        let inner = old_inner.new_with(members, addressbook);

        self.inner.swap(inner);
    }
}

impl<Membership> Clone for DaMembershipHandler<Membership> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<Membership> MembershipHandler for DaMembershipHandler<Membership>
where
    Membership: MembershipHandler<Id = PeerId, NetworkId = u16>,
{
    type Id = PeerId;
    type NetworkId = u16;

    fn new_with(&self, members: Vec<PeerId>, addressbook: HashMap<PeerId, Multiaddr>) -> Self {
        unreachable!("DaMembershipHandler does not support new_with")
    }

    fn membership(&self, id: &Self::Id) -> HashSet<Self::NetworkId> {
        self.inner.load().membership(id)
    }

    fn is_allowed(&self, id: &Self::Id) -> bool {
        self.inner.load().is_allowed(id)
    }

    fn members_of(&self, network_id: &Self::NetworkId) -> HashSet<Self::Id> {
        self.inner.load().members_of(network_id)
    }

    fn members(&self) -> HashSet<Self::Id> {
        self.inner.load().members()
    }

    fn last_subnetwork_id(&self) -> Self::NetworkId {
        self.inner.load().last_subnetwork_id()
    }

    fn get_address(&self, peer_id: &Self::Id) -> Option<Multiaddr> {
        self.inner.load().get_address(peer_id)
    }
}
