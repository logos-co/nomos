use std::collections::{BTreeSet, HashMap};

use broadcast_service::BlockBroadcastService;
use futures::{StreamExt as _, future};
use libp2p::{Multiaddr, PeerId, core::signed_envelope::DecodingError};
use nomos_core::sdp::{Locator, ProviderId, ProviderInfo, SessionNumber};
use nomos_libp2p::ed25519;
use overwatch::services::{ServiceData, relay::OutboundRelay};
use tokio::sync::oneshot;
use tokio_stream::wrappers::WatchStream;

use crate::membership::{
    MembershipAdapter, MembershipAdapterError, PeerMultiaddrStream, SubnetworkPeers,
};

pub struct MembershipServiceAdapter<RuntimeServiceId> {
    relay: OutboundRelay<<BlockBroadcastService<RuntimeServiceId> as ServiceData>::Message>,
}

pub type MembershipProviders = (SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>);

#[async_trait::async_trait]
impl<RuntimeServiceId> MembershipAdapter for MembershipServiceAdapter<RuntimeServiceId>
where
    RuntimeServiceId: Send + Sync + 'static,
{
    type MembershipService = BlockBroadcastService<RuntimeServiceId>;
    type Id = PeerId;

    fn new(relay: OutboundRelay<<Self::MembershipService as ServiceData>::Message>) -> Self {
        Self { relay }
    }

    async fn subscribe(&self) -> Result<PeerMultiaddrStream<Self::Id>, MembershipAdapterError> {
        let (sender, receiver) = oneshot::channel();
        self.relay
            .send(broadcast_service::BlockBroadcastMsg::SubscribeDASession {
                result_sender: sender,
            })
            .await
            .map_err(|(e, _)| MembershipAdapterError::Other(e.into()))?;
        let watch_receiver = receiver
            .await
            .map_err(|e| MembershipAdapterError::Other(e.into()))?;

        let converted_stream = WatchStream::new(watch_receiver)
            .filter(|update| future::ready(update.is_some()))
            .map(|update| {
                let session = update.unwrap();

                let mut peers = HashMap::new();
                let mut provider_mappings = HashMap::new();

                session
                    .providers
                    .into_iter()
                    .filter_map(process_provider)
                    .for_each(|(peer_id, multiaddr, provider_id)| {
                        peers.insert(peer_id, multiaddr);
                        provider_mappings.insert(peer_id, provider_id);
                    });

                SubnetworkPeers {
                    session_id: session.session_number,
                    peers,
                    provider_mappings,
                }
            });

        Ok(Box::pin(converted_stream))
    }
}

fn process_provider(
    (provider_id, info): (ProviderId, ProviderInfo),
) -> Option<(PeerId, Multiaddr, ProviderId)> {
    let locator = info.locators.into_iter().next()?;

    match peer_id_from_provider_id(provider_id.0.as_bytes()) {
        Ok(peer_id) => Some((peer_id, locator.0, provider_id)),
        Err(err) => {
            tracing::warn!(
                "Failed to parse PeerId from provider_id: {:?}, error: {:?}",
                provider_id.0,
                err
            );
            None
        }
    }
}

pub fn peer_id_from_provider_id(pk_raw: &[u8]) -> Result<PeerId, DecodingError> {
    let ed_pub = ed25519::PublicKey::try_from_bytes(pk_raw)?;
    Ok(PeerId::from_public_key(&ed_pub.into()))
}
