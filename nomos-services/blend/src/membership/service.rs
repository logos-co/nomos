use std::collections::BTreeSet;

use futures::StreamExt as _;
use libp2p::{core::signed_envelope::DecodingError, PeerId};
use nomos_blend_message::crypto::Ed25519PublicKey;
use nomos_blend_scheduling::membership::{Membership, Node};
use nomos_core::sdp::{Locator, ProviderId, ServiceType};
use nomos_libp2p::ed25519;
use nomos_membership::{
    backends::MembershipBackendError, MembershipMessage, MembershipSnapshotStream,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use tokio::sync::oneshot;
use tracing::warn;

use crate::membership::MembershipStream;

pub struct Adapter<Service>
where
    Service: ServiceData,
{
    /// A relay to send messages to the membership service.
    relay: OutboundRelay<<Service as ServiceData>::Message>,
    /// A signing public key of the local node, required to
    /// build a [`Membership`] instance.
    signing_public_key: Ed25519PublicKey,
}

#[async_trait::async_trait]
impl<Service> super::Adapter for Adapter<Service>
where
    Service: ServiceData<Message = MembershipMessage>,
{
    type Service = Service;
    type NodeId = PeerId;
    type Error = Error;

    fn new(
        relay: OutboundRelay<<Self::Service as ServiceData>::Message>,
        signing_public_key: Ed25519PublicKey,
    ) -> Self {
        Self {
            relay,
            signing_public_key,
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
                .map(|(_block_number, providers_map)| {
                    providers_map
                        .iter()
                        .filter_map(|(provider_id, locators)| {
                            node_from_provider(provider_id, locators)
                        })
                        .collect::<Vec<_>>()
                })
                .map(move |nodes| Membership::new(&nodes, Some(&signing_public_key))),
        ))
    }
}

impl<Service> Adapter<Service>
where
    Service: ServiceData<Message = MembershipMessage>,
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
fn node_from_provider(
    provider_id: &ProviderId,
    locators: &BTreeSet<Locator>,
) -> Option<Node<PeerId>> {
    let provider_id = provider_id.0;
    let address = locators.first()?.0.clone();
    let id = peer_id_from_provider_id(&provider_id)
        .map_err(|e| {
            warn!("Failed to decode provider_id to peer_id: {e:?}");
        })
        .ok()?;
    let public_key = provider_id
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

fn peer_id_from_provider_id(provider_id: &[u8]) -> Result<PeerId, DecodingError> {
    Ok(PeerId::from_public_key(
        &ed25519::PublicKey::try_from_bytes(provider_id)?.into(),
    ))
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Backend error: {0}")]
    Backend(#[from] MembershipBackendError),
    #[error("Other error: {0}")]
    Other(#[from] DynError),
}
