use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use arc_swap::ArcSwap;
use libp2p::Multiaddr;
use nomos_da_network_core::SubnetworkId;
use subnetworks_assignations::MembershipHandler;

pub trait MembershipStorage {
    fn store(&mut self, block_number: u64, assignations: HashMap<SubnetworkId, HashSet<PeerId>>);
    fn get(&self, block_number: u64) -> Option<HashMap<SubnetworkId, HashSet<PeerId>>>;
}

impl<Membership> DaMembership<Membership>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId> + Clone,
{
    membership: Arc<ArcSwap<Membership>>,
}

pub struct DaMembershipManager<Membership, Storage>
where
    Membership: MembershipHandler + Clone,
    Storage: MembershipStorage,
{
    membership: DaMembership<Membership>,
    storage: Storage,
}

impl<Membership, Storage> DaMembershipManager<Membership, Storage>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId> + Clone,
    Storage: MembershipStorage,
{
    pub fn new(initial_membership: Membership, storage: Storage) -> Self {
        Self {
            membership: Arc::new(ArcSwap::from_pointee(initial_membership)),
            storage,
        }
    }

    pub fn update(&mut self, block_number: u64, new_members: HashMap<PeerId, Multiaddr>) {
        // Get current membership state
        let current_membership = self.membership.load(); // we need similar method in MembershipHandler trait

        // Create new membership with updated data
        let new_membership = current_membership.new_with(new_members);

        // Store the new membership atomically
        self.membership.store(Arc::new(new_membership));

        // Store historical state
        let assignations = self.membership.get_assignations(); // Vec<HashSet<PeerId>>
        self.storage.store(block_number, assignations);
    }

    pub fn get_historic_membership(&self, block_number: u64) -> Option<Membership> {
        // Get historical assignations from storage
        let historical_assignations = self.storage.get(block_number)?; // Vec<HashSet<PeerId>>
                                                                       // recreate membership from Vec<HashSet<PeerId>> or HashMap<Subnet,
                                                                       // HashSet<PeerId>>

        // Reconstruct membership from historical data
        // This is a simplified approach - you might need more sophisticated
        // reconstruction
        let current = self.membership.load();

        // Extract all members from historical assignations
        let all_members: Vec<PeerId> = historical_assignations
            .values()
            .flat_map(|peers| peers.iter().cloned())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        // Create addressbook (you might want to store this separately for historical
        // data)
        let addressbook: HashMap<PeerId, Multiaddr> = all_members
            .iter()
            .filter_map(|peer_id| {
                current
                    .get_address(peer_id)
                    .map(|addr| (peer_id.clone(), addr))
            })
            .collect();

        Some(current.new_with(all_members, addressbook))
    }

    fn get_current_assignations(&self) -> HashMap<SubnetworkId, HashSet<PeerId>> {
        let membership = self.membership.load();
        let last_id = membership.last_subnetwork_id();

        // Reconstruct assignations by iterating through all subnetwork IDs
        (0..=last_id)
            .map(|subnet_id| {
                let members = membership.members_of(&subnet_id);
                (subnet_id, members)
            })
            .filter(|(_, members)| !members.is_empty())
            .collect()
    }

    fn membership(&self) -> DaMembership<Membership> {
        self.membership.clone()
    } 
}

// Implement MembershipHandler for DaMembershipImpl
impl<Membership> MembershipHandler for DaMembership<Membership>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId> + Clone,
{
    type NetworkId = SubnetworkId;
    type Id = PeerId;

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

    fn new(members: Vec<PeerId>, addressbook: HashMap<PeerId, Multiaddr>) -> Self {
        Membership::new(members, addressbook)
    }

    fn update(&self, members: Vec<PeerId>, addressbook: HashMap<PeerId, Multiaddr>) -> Self {
        self.membership.update(memebers, addressbook)
    }

    fn get_address(&self, peer_id: &libp2p::PeerId) -> Option<libp2p::Multiaddr> {
        self.membership.load().get_address(peer_id)
    }
}

// Clone implementation
impl<Membership> Clone for DaMembership<Membership>
where
    Membership: MembershipHandler + Clone,
{
    fn clone(&self) -> Self {
        Self {
            membership: Arc::clone(&self.membership),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub struct MemoryMembershipStorage {
        data: HashMap<u64, HashMap<SubnetworkId, HashSet<PeerId>>>,
    }

    impl MemoryMembershipStorage {
        pub fn new() -> Self {
            Self {
                data: HashMap::new(),
            }
        }
    }

    impl MembershipStorage for MockMembershipStorage {
        fn store(
            &mut self,
            block_number: u64,
            assignations: HashMap<SubnetworkId, HashSet<PeerId>>,
        ) {
            self.data.insert(block_number, assignations);
        }

        fn get(&self, block_number: u64) -> Option<HashMap<SubnetworkId, HashSet<PeerId>>> {
            self.data.get(&block_number).cloned()
        }
    }

    #[test]
    fn test_da_membership_impl() {
        // Create initial membership
        let initial_assignations = vec![
            HashSet::from(["peer1".to_string(), "peer2".to_string()]),
            HashSet::from(["peer3".to_string()]),
        ];
        let initial_addressbook = HashMap::from([
            ("peer1".to_string(), "addr1".to_string()),
            ("peer2".to_string(), "addr2".to_string()),
            ("peer3".to_string(), "addr3".to_string()),
        ]);

        let membership = SimpleMembership::new(initial_assignations, initial_addressbook);
        let storage = MemoryMembershipStorage::new();

        // Create DaMembershipImpl
        let mut da_membership = DaMembershipManager::new(membership, storage);

        // Test current membership
        let current = da_membership.get_current_membership();
        assert_eq!(current.members().len(), 3);

        // Test update
        let new_members = HashMap::from([("peer4".to_string(), "addr4".to_string())]);
        da_membership.update(100, new_members);

        // Test historical retrieval
        let historical = da_membership.get_historic_membership(100);
        assert!(historical.is_some());
    }
}