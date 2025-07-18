pub mod adapters;
use std::collections::HashSet;

use blake2::{digest::Update as BlakeUpdate, Blake2b512, Digest as _};
use nomos_core::block::BlockNumber;
use nomos_utils::blake_rng::BlakeRng;
use overwatch::services::{relay::OutboundRelay, ServiceData};
use rand::SeedableRng as _;
use subnetworks_assignations::{MembershipCreator, MembershipHandler};

use crate::membership::{handler::DaMembershipHandler, Assignations};

pub trait MembershipStorageAdapter<Id, NetworkId> {
    type StorageService: ServiceData;

    fn new(relay: OutboundRelay<<Self::StorageService as ServiceData>::Message>) -> Self;

    fn store(&self, block_number: BlockNumber, assignations: Assignations<Id, NetworkId>);
    fn get(&self, block_number: BlockNumber) -> Option<Assignations<Id, NetworkId>>;

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

    pub fn update(&self, block_number: BlockNumber, new_members: HashSet<Membership::Id>) {
        let mut hasher = Blake2b512::default();
        BlakeUpdate::update(&mut hasher, block_number.to_le_bytes().as_slice());
        let seed: [u8; 64] = hasher.finalize().into();
        let mut rng = BlakeRng::from_seed(seed.into());
        let updated_membership = self.handler.membership().update(new_members, &mut rng);
        let assignations = updated_membership.subnetworks();

        tracing::debug!("Updating membership at block {block_number} with {assignations:?}");
        self.handler.update(updated_membership);
        self.adapter.store(block_number, assignations);
    }

    pub fn get_historic_membership(&self, block_number: BlockNumber) -> Option<Membership> {
        let assignations = self.adapter.get(block_number)?;
        Some(self.handler.membership().init(assignations))
    }
}
