use std::collections::HashMap;

use libp2p::{Multiaddr, PeerId};
use nomos_da_network_core::SubnetworkId;
use subnetworks_assignations::MembershipCreator;

use crate::membership::{
    adapter::MembershipAdapter, handler::DaMembershipHandler, MembershipStorage,
};

pub struct MockMembershipAdapter<Membership, Storage>
where
    Membership: MembershipCreator + Clone,
    Storage: MembershipStorage,
{
    handler: DaMembershipHandler<Membership>,
    storage: Storage,
}

impl<Membership, Storage> MembershipAdapter<Membership, Storage>
    for MockMembershipAdapter<Membership, Storage>
where
    Membership: MembershipCreator<NetworkId = SubnetworkId, Id = PeerId> + Clone,
    Storage: MembershipStorage,
{
    fn new(handler: DaMembershipHandler<Membership>, storage: Storage) -> Self {
        Self { handler, storage }
    }

    fn update(&self, block_number: u64, new_members: HashMap<PeerId, Multiaddr>) {
        let updated_membership = self.handler.membership().update(new_members);
        let assignations = updated_membership.subnetworks();

        self.handler.update(updated_membership);
        self.storage.store(block_number, assignations);
    }

    fn get_historic_membership(&self, block_number: u64) -> Option<Membership> {
        let assignations = self.storage.get(block_number)?;
        Some(self.handler.membership().init(assignations))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::{HashMap, HashSet},
    };

    use libp2p::{Multiaddr, PeerId};
    use nomos_da_network_core::SubnetworkId;
    use subnetworks_assignations::{MembershipCreator, MembershipHandler};

    use super::MockMembershipAdapter;
    use crate::membership::{
        adapter::MembershipAdapter as _, handler::DaMembershipHandler, Assignations,
        MembershipStorage,
    };

    #[derive(Default, Clone)]
    struct MockMembership {
        assignations: Assignations,
    }

    impl MembershipHandler for MockMembership {
        type NetworkId = SubnetworkId;
        type Id = PeerId;

        fn membership(&self, _id: &Self::Id) -> HashSet<Self::NetworkId> {
            unimplemented!()
        }

        fn is_allowed(&self, _id: &Self::Id) -> bool {
            unimplemented!()
        }

        fn members_of(&self, _network_id: &Self::NetworkId) -> HashSet<Self::Id> {
            unimplemented!()
        }

        fn members(&self) -> HashSet<Self::Id> {
            unimplemented!()
        }

        fn last_subnetwork_id(&self) -> Self::NetworkId {
            unimplemented!()
        }

        fn get_address(&self, _peer_id: &Self::Id) -> Option<Multiaddr> {
            unimplemented!()
        }

        fn subnetworks(&self) -> HashMap<Self::NetworkId, HashSet<Self::Id>> {
            self.assignations.clone()
        }
    }

    impl MembershipCreator for MockMembership {
        fn init(&self, peer_assignments: HashMap<Self::NetworkId, HashSet<PeerId>>) -> Self {
            Self {
                assignations: peer_assignments,
            }
        }

        fn update(&self, new_peer_addresses: HashMap<Self::Id, Multiaddr>) -> Self {
            let mut assignations = HashMap::new();
            assignations.insert(99, new_peer_addresses.keys().copied().collect());

            Self { assignations }
        }
    }

    struct MockBackend<Membership>
    where
        Membership: MembershipHandler,
    {
        membership: Membership,
    }

    #[derive(Default)]
    struct MockStorage {
        storage: RefCell<HashMap<u64, Assignations>>,
    }

    impl MembershipStorage for MockStorage {
        fn store(&self, block_number: u64, assignations: Assignations) {
            let mut storage = self.storage.borrow_mut();
            storage.insert(block_number, assignations);
        }

        fn get(&self, block_number: u64) -> Option<Assignations> {
            self.storage.borrow().get(&block_number).cloned()
        }
    }

    struct MockService {
        backend: MockBackend<DaMembershipHandler<MockMembership>>,
        membership: DaMembershipHandler<MockMembership>,
    }

    impl MockService {
        fn init() -> Self {
            // DaMembershipHandler is passed to the backend as MembershipHandler.
            // In Adapter, DaMembershipHandler exposes the `update` method, which allows
            // backend to get the updates, but only Adapter to initiate changes.
            let membership = DaMembershipHandler::new(MockMembership::default());
            let backend = MockBackend {
                membership: membership.clone(),
            };

            Self {
                backend,
                membership,
            }
        }

        fn run(&self) {
            // Here real adapter would subscribe to the membership service for declaration
            // info updates.
            let adapter = MockMembershipAdapter::new(
                self.membership.clone(),
                MockStorage::default(), // Here a handle to real storage would be passed.
            );

            // Instead of loop we imitate one iteration by changing updating
            // members via the membership, the changes should also
            // appear in the backend, where clone of DaMembershipHandler
            // was passed in the init method.
            let mut update = HashMap::new();
            update.insert(PeerId::random(), Multiaddr::empty());

            adapter.update(1, update);
        }
    }

    #[test]
    fn test_adapter_usage() {
        // Test demonstrates the usage of DaMembershipHandler in Overwatch service.
        //
        // Network backend with swarm is initialized in then service init
        // method. Adapter will be initialized in the run method, because it needs
        // handle to the membership service. To allow backend to read the current
        // membership and adapter to update the membership, a shared DaMembershipHandler
        // was created.
        //
        // It's initialized inside init method, and then passed as clone to the adapter
        // in the run method.
        //
        // To see how DaMembership is initialized and later cloned please check out
        // MockService::init and MockService::run methods.
        let service = MockService::init();
        service.run();

        let backend_assignations = service.backend.membership.subnetworks();
        assert_eq!(backend_assignations.len(), 1);
    }
}
