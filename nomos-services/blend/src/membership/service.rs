use std::{collections::BTreeSet, hash::Hash, marker::PhantomData};

use futures::StreamExt as _;
use nomos_blend_message::crypto::keys::Ed25519PublicKey;
use nomos_blend_scheduling::membership::{Membership, Node};
use nomos_core::sdp::{Locator, ProviderId, ServiceType};
use nomos_membership_service::{
    MembershipMessage, MembershipSnapshotStream, backends::MembershipBackendError,
};
use overwatch::{
    DynError,
    services::{ServiceData, relay::OutboundRelay},
};
use tokio::sync::oneshot;
use tracing::warn;

use crate::membership::{MembershipStream, ServiceMessage, node_id};

pub struct Adapter<Service, NodeId>
where
    Service: ServiceData,
{
    /// A relay to send messages to the membership service.
    relay: OutboundRelay<<Service as ServiceData>::Message>,
    /// A signing public key of the local node, required to
    /// build a [`Membership`] instance.
    signing_public_key: Ed25519PublicKey,
    _phantom: PhantomData<NodeId>,
}

#[async_trait::async_trait]
impl<Service, NodeId> super::Adapter for Adapter<Service, NodeId>
where
    Service: ServiceData<Message = MembershipMessage>,
    NodeId: node_id::TryFrom + Clone + Hash + Eq + Sync,
{
    type Service = Service;
    type NodeId = NodeId;
    type Error = Error;

    fn new(
        relay: OutboundRelay<ServiceMessage<Self>>,
        signing_public_key: Ed25519PublicKey,
    ) -> Self {
        Self {
            relay,
            signing_public_key,
            _phantom: PhantomData,
        }
    }

    /// Subscribe to membership updates.
    ///
    /// It returns a stream of [`Membership`] instances,
    async fn subscribe(&self) -> Result<MembershipStream<Self::NodeId>, Self::Error> {
        let signing_public_key = self.signing_public_key;
        Ok(Box::pin(
            self.subscribe_stream(ServiceType::BlendNetwork)
                .await?
                .map(|(_, providers_map)| {
                    providers_map
                        .iter()
                        .filter_map(|(provider_id, locators)| {
                            node_from_provider::<NodeId>(provider_id, locators)
                        })
                        .collect::<Vec<_>>()
                })
                .map(move |nodes| Membership::new(&nodes, &signing_public_key)),
        ))
    }
}

impl<Service, NodeId> Adapter<Service, NodeId>
where
    Service: ServiceData<Message = MembershipMessage>,
    NodeId: Sync,
{
    /// Subscribe to membership updates for the given service type.
    async fn subscribe_stream(
        &self,
        service_type: ServiceType,
    ) -> Result<MembershipSnapshotStream, Error> {
        let (sender, receiver) = oneshot::channel();

        self.relay
            .send(MembershipMessage::Subscribe {
                service_type,
                result_sender: sender,
            })
            .await
            .map_err(|(e, _)| Error::Other(e.into()))?;

        receiver
            .await
            .map_err(|e| Error::Other(e.into()))?
            .map_err(Error::Backend)
    }
}

/// Builds a [`Node`] from a [`ProviderId`] and a set of [`Locator`]s.
/// Returns [`None`] if the locators set is empty or if the provider ID cannot
/// be decoded.
fn node_from_provider<NodeId>(
    provider_id: &ProviderId,
    locators: &BTreeSet<Locator>,
) -> Option<Node<NodeId>>
where
    NodeId: node_id::TryFrom,
{
    let provider_id = provider_id.0.as_bytes();
    let address = locators.first()?.0.clone();
    let id = NodeId::try_from_provider_id(provider_id)
        .map_err(|e| {
            warn!("Failed to decode provider_id to node ID: {e:?}");
        })
        .ok()?;
    let public_key = (*provider_id)
        .try_into()
        .map_err(|e| {
            warn!("Failed to decode provider_id to public_key: {e:?}");
        })
        .ok()?;
    Some(Node {
        id,
        address,
        public_key,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Backend error: {0}")]
    Backend(#[from] MembershipBackendError),
    #[error("Other error: {0}")]
    Other(#[from] DynError),
}
