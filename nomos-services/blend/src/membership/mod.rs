pub mod node_id;
pub mod service;

use std::pin::Pin;

use futures::Stream;
use nomos_blend_message::crypto::keys::Ed25519PublicKey;
use nomos_blend_scheduling::membership::Membership;
use overwatch::services::{ServiceData, relay::OutboundRelay};

#[derive(Clone, Debug)]
/// All information about core nodes available for each session.
pub struct SessionInfo<NodeId> {
    /// The list of core nodes and their public information.
    pub membership: Membership<NodeId>,
    /// The session number.
    pub session_number: u64,
}

#[cfg(test)]
impl<NodeId> From<Membership<NodeId>> for SessionInfo<NodeId> {
    fn from(value: Membership<NodeId>) -> Self {
        Self {
            membership: value,
            session_number: 1,
        }
    }
}

pub type MembershipStream<NodeId> =
    Pin<Box<dyn Stream<Item = SessionInfo<NodeId>> + Send + Sync + 'static>>;

pub type ServiceMessage<MembershipAdapter> =
    <<MembershipAdapter as Adapter>::Service as ServiceData>::Message;

/// An adapter for the membership service.
#[async_trait::async_trait]
pub trait Adapter {
    type Service: ServiceData;
    type NodeId;
    type Error: std::error::Error;

    fn new(
        relay: OutboundRelay<ServiceMessage<Self>>,
        signing_public_key: Ed25519PublicKey,
    ) -> Self;

    /// Subscribe to membership updates.
    async fn subscribe(&self) -> Result<MembershipStream<Self::NodeId>, Self::Error>;
}
