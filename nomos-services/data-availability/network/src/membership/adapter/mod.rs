use std::collections::HashMap;

use libp2p::{Multiaddr, PeerId};

use crate::membership::handler::DaMembershipHandler;

pub mod mock;

pub trait MembershipAdapter<Membership, Storage> {
    fn new(
        membership: Membership,
        handler: DaMembershipHandler<Membership>,
        storage: Storage,
    ) -> Self;

    fn update(&mut self, block_number: u64, new_members: HashMap<PeerId, Multiaddr>);
    fn get_historic_membership(&self, block_number: u64) -> Option<Membership>;
}
