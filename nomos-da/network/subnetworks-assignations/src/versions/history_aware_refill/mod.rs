use std::{collections::HashSet, fmt::Debug, hash::Hash};

use libp2p_identity::PeerId;

use crate::{MembershipHandler, SubnetworkAssignations};

pub mod assignations;
mod participant;
mod subnetwork;

pub struct HistoryAware<Id> {
    assignations: assignations::Assignations<Id>,
}

impl<Id> MembershipHandler for HistoryAware<Id>
where
    Id: Ord + PartialOrd + Eq + Copy + Hash + Debug,
{
    type NetworkId = u16;
    type Id = Id;

    fn membership(&self, id: &Self::Id) -> HashSet<Self::NetworkId> {
        self.assignations
            .iter()
            .enumerate()
            .filter_map(|(subnetwork_id, subnetwork)| {
                subnetwork
                    .contains(id)
                    .then_some(subnetwork_id as Self::NetworkId)
            })
            .collect()
    }

    fn is_allowed(&self, id: &Self::Id) -> bool {
        for subnetwork in &self.assignations {
            if subnetwork.contains(id) {
                return true;
            }
        }
        false
    }

    fn members_of(&self, network_id: &Self::NetworkId) -> HashSet<Self::Id> {
        self.assignations
            .get(*network_id as usize)
            .expect("Subntework not found")
            .iter()
            .copied()
            .collect()
    }

    fn members(&self) -> HashSet<Self::Id> {
        self.assignations.iter().flatten().copied().collect()
    }

    fn last_subnetwork_id(&self) -> Self::NetworkId {
        self.assignations.len() as Self::NetworkId
    }

    fn subnetworks(&self) -> SubnetworkAssignations<Self::NetworkId, Self::Id> {
        self.assignations
            .iter()
            .enumerate()
            .map(|(id, subnetwork)| {
                (
                    id as Self::NetworkId,
                    subnetwork.into_iter().copied().collect::<HashSet<_>>(),
                )
            })
            .collect()
    }
}
