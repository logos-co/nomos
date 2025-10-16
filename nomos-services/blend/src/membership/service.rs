use std::{hash::Hash, marker::PhantomData};

use broadcast_service::BlockBroadcastMsg;
use futures::{StreamExt as _, future::ready};
use nomos_blend_message::crypto::keys::Ed25519PublicKey;
use nomos_blend_scheduling::membership::{Membership, Node};
use nomos_core::sdp::{Locator, ProviderId, ProviderInfo, SessionUpdate};
use overwatch::{
    DynError,
    services::{ServiceData, relay::OutboundRelay},
};
use tokio::sync::{oneshot, watch};
use tokio_stream::wrappers::WatchStream;
use tracing::warn;

use crate::membership::{MembershipStream, ServiceMessage, SessionInfo, node_id};

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
    Service: ServiceData<Message = BlockBroadcastMsg>,
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
        let blend_stream = WatchStream::new(self.subscribe_to_blend_stream().await?);

        Ok(Box::pin(
            blend_stream
                // Do not yield if the initial value is `None`. We assume once `Some`, no `None`
                // will ever be returned by this channel.
                .filter_map(ready)
                .map(
                    move |SessionUpdate {
                              providers,
                              session_number,
                          }| {
                        let nodes = providers
                            .iter()
                            .filter_map(|(provider_id, ProviderInfo { locators, .. })| {
                                node_from_provider::<NodeId>(provider_id, locators)
                            })
                            .collect::<Vec<_>>();
                        let membership = Membership::new(&nodes, &signing_public_key);
                        SessionInfo {
                            membership,
                            session_number,
                        }
                    },
                ),
        ))
    }
}

impl<Service, NodeId> Adapter<Service, NodeId>
where
    Service: ServiceData<Message = BlockBroadcastMsg>,
    NodeId: Sync,
{
    /// Subscribe to membership updates for Blend.
    async fn subscribe_to_blend_stream(
        &self,
    ) -> Result<watch::Receiver<Option<SessionUpdate>>, Error> {
        let (sender, receiver) = oneshot::channel();

        self.relay
            .send(BlockBroadcastMsg::SubscribeBlendSession {
                result_sender: sender,
            })
            .await
            .map_err(|(e, _)| Error::Other(e.into()))?;

        receiver.await.map_err(|e| Error::Other(e.into()))
    }
}

/// Builds a [`Node`] from a [`ProviderId`] and a set of [`Locator`]s.
/// Returns [`None`] if the locators set is empty or if the provider ID cannot
/// be decoded.
fn node_from_provider<NodeId>(
    provider_id: &ProviderId,
    locators: &[Locator],
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
    #[error("Other error: {0}")]
    Other(#[from] DynError),
}
