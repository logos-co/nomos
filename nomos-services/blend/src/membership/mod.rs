pub mod node_id;
pub mod service;

use std::pin::Pin;

use futures::Stream;
use nomos_blend_message::crypto::keys::Ed25519PublicKey;
use nomos_blend_scheduling::membership::Membership;
use nomos_core::crypto::ZkHash;
use overwatch::services::{ServiceData, relay::OutboundRelay};

#[derive(Clone, Debug)]
pub struct MembershipInfo<NodeId> {
    pub membership: Membership<NodeId>,
    pub zk_root: ZkHash,
    pub session_number: u64,
}

pub type MembershipStream<NodeId> =
    Pin<Box<dyn Stream<Item = MembershipInfo<NodeId>> + Send + Sync + 'static>>;

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
