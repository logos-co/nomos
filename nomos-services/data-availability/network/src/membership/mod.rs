pub mod adapter;
pub mod handler;

use std::collections::{HashMap, HashSet};

use libp2p::PeerId;
use nomos_da_network_core::SubnetworkId;

pub type Assignations = HashMap<SubnetworkId, HashSet<PeerId>>;

pub trait MembershipStorage {
    fn store(&self, block_number: u64, assignations: Assignations);
    fn get(&self, block_number: u64) -> Option<Assignations>;
}
