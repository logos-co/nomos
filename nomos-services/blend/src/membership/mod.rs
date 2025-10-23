pub mod node_id;
pub mod service;

use std::pin::Pin;

use futures::Stream;
use nomos_blend_message::crypto::keys::Ed25519PublicKey;
use nomos_blend_scheduling::membership::Membership;
use nomos_core::{crypto::ZkHash, mantle::keys::PublicKey};
use overwatch::services::{ServiceData, relay::OutboundRelay};
use poq::CorePathAndSelectors;

#[derive(Clone, Debug)]
pub struct MembershipInfo<NodeId> {
    pub membership: Membership<NodeId>,
    pub zk: ZkInfo,
    pub session_number: u64,
}

impl<NodeId> MembershipInfo<NodeId> {
    #[cfg(test)]
    #[must_use]
    pub fn from_membership_and_session_number(
        membership: Membership<NodeId>,
        session_number: u64,
    ) -> Self {
        Self {
            membership,
            session_number,
            zk: ZkInfo::default(),
        }
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(Default))]
pub struct ZkInfo {
    pub root: ZkHash,
    pub core_and_path_selectors: Option<CorePathAndSelectors>,
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
        zk_public_key: Option<PublicKey>,
    ) -> Self;

    /// Subscribe to membership updates.
    async fn subscribe(&self) -> Result<MembershipStream<Self::NodeId>, Self::Error>;
}
