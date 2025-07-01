pub mod adapters;
use std::collections::HashMap;

use libp2p::Multiaddr;
use nomos_core::block::BlockNumber;
use overwatch::services::{relay::OutboundRelay, ServiceData};
use subnetworks_assignations::{MembershipCreator, MembershipHandler};

use crate::membership::{handler::DaMembershipHandler, Assignations};

pub trait MembershipStorageAdapter<Id, NetworkId> {
    type StorageService: ServiceData;

    fn new(relay: OutboundRelay<<Self::StorageService as ServiceData>::Message>) -> Self;

    fn store(
        &self,
        block_number: BlockNumber,
        assignations: Assignations<Id, NetworkId>,
        addressbook: HashMap<Id, Multiaddr>,
    );
    fn get(
        &self,
        block_number: BlockNumber,
    ) -> Option<(Assignations<Id, NetworkId>, HashMap<Id, Multiaddr>)>;

    fn prune(&self);
}

pub struct MembershipStorage<Adapter, Membership> {
    adapter: Adapter,
    handler: DaMembershipHandler<Membership>,
}

impl<Adapter, Membership> MembershipStorage<Adapter, Membership>
where
    Adapter: MembershipStorageAdapter<
            <Membership as MembershipHandler>::Id,
            <Membership as MembershipHandler>::NetworkId,
        > + Send
        + Sync,
    Membership: MembershipCreator + MembershipHandler + Clone + Send + Sync,
    Membership::Id: Send + Sync,
{
    pub const fn new(adapter: Adapter, handler: DaMembershipHandler<Membership>) -> Self {
        Self { adapter, handler }
    }

    pub fn update(
        &self,
        block_number: BlockNumber,
        new_members: HashMap<Membership::Id, Multiaddr>,
    ) {
        let updated_membership = self.handler.membership().update(new_members);
        let assignations = updated_membership.subnetworks();
        let addressbook = updated_membership.addressbook();

        tracing::debug!("Updating membership at block {block_number} with {assignations:?}");
        self.handler.update(updated_membership);
        self.adapter.store(block_number, assignations, addressbook);
    }

    pub fn get_historic_membership(&self, block_number: BlockNumber) -> Option<Membership> {
        let (assignations, addressbook) = self.adapter.get(block_number)?;
        Some(self.handler.membership().init(assignations, addressbook))
    }
}
