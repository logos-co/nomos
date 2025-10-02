pub mod adapters;
pub mod handler;

use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
};

use futures::Stream;
use libp2p::Multiaddr;
use nomos_core::block::SessionNumber;
use nomos_membership_service::backends::MembershipBackendError;
use overwatch::{
    DynError,
    services::{ServiceData, relay::OutboundRelay},
};
use thiserror::Error;

pub type Assignations<Id, NetworkId> = HashMap<NetworkId, HashSet<Id>>;

pub type SubnetworkPeers<Id> = (SessionNumber, HashMap<Id, Multiaddr>);

pub type PeerMultiaddrStream<Id> =
    Pin<Box<dyn Stream<Item = SubnetworkPeers<Id>> + Send + Sync + 'static>>;

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
    type Id;

    fn new(relay: OutboundRelay<<Self::MembershipService as ServiceData>::Message>) -> Self;

    async fn subscribe(&self) -> Result<PeerMultiaddrStream<Self::Id>, MembershipAdapterError>;
}
