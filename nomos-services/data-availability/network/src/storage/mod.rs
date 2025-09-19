pub mod adapters;
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use blake2::{Blake2b512, Digest as _, digest::Update as BlakeUpdate};
use multiaddr::Multiaddr;
use nomos_core::block::SessionNumber;
use nomos_utils::blake_rng::BlakeRng;
use overwatch::{
    DynError,
    services::{ServiceData, relay::OutboundRelay},
};
use rand::SeedableRng as _;
use subnetworks_assignations::{MembershipCreator, MembershipHandler};

use crate::{
    addressbook::{AddressBookMut, AddressBookSnapshot},
    membership::{Assignations, handler::DaMembershipHandler},
};

#[async_trait::async_trait]
pub trait MembershipStorageAdapter<Id, NetworkId> {
    type StorageService: ServiceData;

    fn new(relay: OutboundRelay<<Self::StorageService as ServiceData>::Message>) -> Self;

    async fn store(
        &self,
        session_id: SessionNumber,
        assignations: Assignations<Id, NetworkId>,
    ) -> Result<(), DynError>;
    async fn get(
        &self,
        session_id: SessionNumber,
    ) -> Result<Option<Assignations<Id, NetworkId>>, DynError>;

    async fn store_addresses(&self, ids: HashMap<Id, Multiaddr>) -> Result<(), DynError>;
    async fn get_address(&self, id: Id) -> Result<Option<Multiaddr>, DynError>;
    async fn prune(&self);
}

pub struct MembershipStorage<MembershipAdapter, Membership, AddressBook> {
    membership_adapter: MembershipAdapter,
    membership_handler: DaMembershipHandler<Membership>,
    addressbook: AddressBook,
}

impl<MembershipAdapter, Membership, AddressBook>
    MembershipStorage<MembershipAdapter, Membership, AddressBook>
where
    MembershipAdapter: MembershipStorageAdapter<
            <Membership as MembershipHandler>::Id,
            <Membership as MembershipHandler>::NetworkId,
        > + Send
        + Sync,
    Membership: MembershipCreator + Clone + Send + Sync,
    Membership::Id: Send + Sync + Clone + Copy + Eq + Hash,
    AddressBook: AddressBookMut<Id = Membership::Id> + Send + Sync,
{
    pub const fn new(
        membership_adapter: MembershipAdapter,
        membership_handler: DaMembershipHandler<Membership>,
        addressbook: AddressBook,
    ) -> Self {
        Self {
            membership_adapter,
            membership_handler,
            addressbook,
        }
    }

    pub async fn update(
        &self,
        session_id: SessionNumber,
        new_members: AddressBookSnapshot<Membership::Id>,
    ) -> Result<(), DynError> {
        let mut hasher = Blake2b512::default();
        BlakeUpdate::update(&mut hasher, session_id.to_le_bytes().as_slice());
        let seed: [u8; 64] = hasher.finalize().into();

        let update: HashSet<Membership::Id> = new_members.keys().copied().collect();

        // Scope the RNG so it's dropped before the await
        let (updated_membership, assignations) = {
            let mut rng = BlakeRng::from_seed(seed.into());
            let updated_membership = self
                .membership_handler
                .membership()
                .update(update, &mut rng);
            let assignations = updated_membership.subnetworks();
            (updated_membership, assignations)
        };

        tracing::debug!("Updating membership at session {session_id} with {assignations:?}");

        // update in-memory latest membership
        self.membership_handler.update(updated_membership);
        self.addressbook.update(new_members.clone());

        // update membership storage
        self.membership_adapter
            .store(session_id, assignations)
            .await?;
        self.membership_adapter.store_addresses(new_members).await
    }

    pub async fn get_historic_membership(
        &self,
        session_id: SessionNumber,
    ) -> Result<Option<Membership>, DynError> {
        let mut membership = None;

        if let Some(assignations) = self.membership_adapter.get(session_id).await? {
            membership = Some(self.membership_handler.membership().init(assignations));
        }

        if membership.is_none() {
            tracing::debug!("No membership found for session {session_id}");
            return Ok(None);
        }

        Ok(Some(membership.unwrap()))
    }
}
