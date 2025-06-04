use std::collections::HashMap;

use libp2p::{Multiaddr, PeerId};
use nomos_da_network_core::SubnetworkId;
use subnetworks_assignations::MembershipCreator;

use crate::membership::{handler::DaMembershipHandler, MembershipStorage};

pub struct MockMembershipAdapter<Membership, Storage>
where
    Membership: MembershipCreator + Clone,
    Storage: MembershipStorage,
{
    membership: Membership,
    handler: DaMembershipHandler<Membership>,
    storage: Storage,
}

impl<Membership, Storage> MockMembershipAdapter<Membership, Storage>
where
    Membership: MembershipCreator<NetworkId = SubnetworkId, Id = PeerId> + Clone,
    Storage: MembershipStorage,
{
    pub fn new(
        membership: Membership,
        handler: DaMembershipHandler<Membership>,
        storage: Storage,
    ) -> Self {
        Self {
            membership,
            handler,
            storage,
        }
    }

    pub fn update(&mut self, block_number: u64, new_members: HashMap<PeerId, Multiaddr>) {
        todo!()
    }

    pub fn get_historic_membership(&self, block_number: u64) -> Option<Membership> {
        todo!()
    }
}
