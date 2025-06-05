use std::{collections::HashSet, sync::Arc};

use arc_swap::ArcSwap;
use subnetworks_assignations::MembershipHandler;

pub struct DaMembershipHandler<Membership> {
    membership: Arc<ArcSwap<Membership>>,
}

impl<Membership> DaMembershipHandler<Membership> {
    pub fn new(membership: Membership) -> Self {
        Self {
            membership: Arc::new(ArcSwap::from_pointee(membership)),
        }
    }

    pub fn update(&self, membership: Membership) {
        // `ArcSwap::store` uses `ArcSwap::swap` internally and in addition drops the
        // previously stored value.
        self.membership.store(Arc::new(membership));
    }
}

// Implement MembershipHandler for DaMembershipImpl
impl<Membership> MembershipHandler for DaMembershipHandler<Membership>
where
    Membership: MembershipHandler + Clone,
{
    type NetworkId = Membership::NetworkId;
    type Id = Membership::Id;

    fn membership(&self, id: &Self::Id) -> HashSet<Self::NetworkId> {
        self.membership.load().membership(id)
    }

    fn is_allowed(&self, id: &Self::Id) -> bool {
        self.membership.load().is_allowed(id)
    }

    fn members_of(&self, network_id: &Self::NetworkId) -> HashSet<Self::Id> {
        self.membership.load().members_of(network_id)
    }

    fn members(&self) -> HashSet<Self::Id> {
        self.membership.load().members()
    }

    fn last_subnetwork_id(&self) -> Self::NetworkId {
        self.membership.load().last_subnetwork_id()
    }

    fn get_address(&self, peer_id: &libp2p::PeerId) -> Option<libp2p::Multiaddr> {
        self.membership.load().get_address(peer_id)
    }
}

impl<Membership> Clone for DaMembershipHandler<Membership>
where
    Membership: MembershipHandler,
{
    fn clone(&self) -> Self {
        Self {
            membership: Arc::clone(&self.membership),
        }
    }
}
