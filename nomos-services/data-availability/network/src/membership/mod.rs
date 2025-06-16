pub mod adapters;
pub mod handler;

use std::collections::{HashMap, HashSet};

use libp2p::PeerId;
use nomos_da_network_core::SubnetworkId;
use nomos_membership::{backends::MembershipBackendError, MembershipSnapshotStream};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use thiserror::Error;

pub type Assignations<Id, NetworkId> = HashMap<NetworkId, HashSet<Id>>;

#[derive(Error, Debug)]
pub enum MembershipAdapterError {
    #[error("Backend error: {0}")]
    Backend(#[from] MembershipBackendError),

    #[error("Other error: {0}")]
    Other(#[from] DynError),
}

#[async_trait::async_trait]
pub trait MembershipAdapter {
    type MembershipService: ServiceData;

    fn new(relay: OutboundRelay<<Self::MembershipService as ServiceData>::Message>) -> Self;

    async fn subscribe(&self) -> Result<MembershipSnapshotStream, MembershipAdapterError>;
}
