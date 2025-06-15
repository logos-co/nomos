pub mod mock;

use std::sync::Arc;

use nomos_membership::backends::MembershipBackendError;
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use thiserror::Error;

use crate::membership::handler::DaMembershipHandler;

#[derive(Error, Debug)]
pub enum MembershipAdapterError {
    #[error("Backend error: {0}")]
    Backend(#[from] MembershipBackendError),

    #[error("Other error: {0}")]
    Other(#[from] DynError),
}

#[async_trait::async_trait]
pub trait MembershipAdapter<Membership, Storage> {
    type MembershipService: ServiceData;

    fn new(
        relay: OutboundRelay<<Self::MembershipService as ServiceData>::Message>,
        handler: DaMembershipHandler<Membership>,
        storage: Storage,
    ) -> Self;

    async fn bootstrap(self: Arc<Self>) -> Result<(), MembershipAdapterError>;
    async fn get_historic_membership(&self, block_number: u64) -> Option<Membership>;
}
