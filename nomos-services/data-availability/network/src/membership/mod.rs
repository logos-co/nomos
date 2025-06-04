pub mod adapter;
pub mod handler;

use std::collections::{HashMap, HashSet};

use libp2p::PeerId;
use nomos_da_network_core::SubnetworkId;

pub type Asssignations = HashMap<SubnetworkId, HashSet<PeerId>>;

pub trait MembershipStorage {
    fn store(&mut self, block_number: u64, assignations: Asssignations);
    fn get(&self, block_number: u64) -> Option<HashMap<SubnetworkId, HashSet<PeerId>>>;
}
