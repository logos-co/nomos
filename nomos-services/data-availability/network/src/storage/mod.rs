pub mod adapters;
use nomos_core::block::BlockNumber;

use crate::membership::Assignations;

pub trait MembershipStorage {
    fn store(&self, block_number: BlockNumber, assignations: Assignations);
    fn get(&self, block_number: BlockNumber) -> Option<Assignations>;
}
