use std::{collections::HashSet, fmt::Debug, hash::Hash};

use rand::RngCore;

use crate::{
    versions::history_aware_refill::assignations::HistoryAwareRefill, MembershipCreator,
    MembershipHandler, SubnetworkAssignations, SubnetworkId,
};

pub mod assignations;
mod participant;
mod subnetwork;

#[derive(Clone)]
pub struct HistoryAware<Id> {
    assignations: assignations::Assignations<Id>,
    subnetwork_size: usize,
    replication_factor: usize,
}

impl<Id> MembershipHandler for HistoryAware<Id>
where
    Id: Ord + PartialOrd + Eq + Copy + Hash + Debug,
{
    type NetworkId = SubnetworkId;
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
                    subnetwork.iter().copied().collect::<HashSet<_>>(),
                )
            })
            .collect()
    }
}

impl<Id> MembershipCreator for HistoryAware<Id>
where
    for<'id> Id: Ord + PartialOrd + Eq + Copy + Hash + Debug + 'id,
{
    fn init(&self, peer_addresses: SubnetworkAssignations<Self::NetworkId, Self::Id>) -> Self {
        let mut assignations: Vec<_> = peer_addresses.into_iter().collect();
        assignations.sort_by_key(|(id, _)| *id);
        let assignations: Vec<_> = assignations
            .into_iter()
            .map(|(_, ids)| ids.into_iter().collect())
            .collect();
        Self {
            assignations,
            subnetwork_size: self.subnetwork_size,
            replication_factor: self.replication_factor,
        }
    }

    fn update<Rng: RngCore>(&self, new_nodes: HashSet<Self::Id>, rng: &mut Rng) -> Self {
        let Self {
            assignations,
            subnetwork_size,
            replication_factor,
        } = self.clone();
        let new_nodes: Vec<_> = new_nodes.into_iter().collect();
        let assignations = HistoryAwareRefill::calculate_subnetwork_assignations(
            &new_nodes,
            assignations,
            replication_factor,
            rng,
        );
        Self {
            assignations,
            subnetwork_size,
            replication_factor,
        }
    }
}
