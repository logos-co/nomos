pub mod adapters;
pub mod handler;

use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
};

use futures::Stream;
use libp2p::Multiaddr;
use nomos_core::block::BlockNumber;
use nomos_membership::backends::MembershipBackendError;
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use thiserror::Error;

pub type Assignations<Id, NetworkId> = HashMap<NetworkId, HashSet<Id>>;

pub type SubnetworkPeers<Id> = (BlockNumber, HashMap<Id, Multiaddr>);

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
