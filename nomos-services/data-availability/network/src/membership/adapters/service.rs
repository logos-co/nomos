use std::{collections::HashMap, marker::PhantomData};

use futures::StreamExt as _;
use libp2p::{Multiaddr, PeerId, core::signed_envelope::DecodingError};
use nomos_core::sdp::{ProviderId, ServiceType};
use nomos_libp2p::ed25519;
use nomos_membership_service::{MembershipMessage, MembershipService, MembershipSnapshotStream};
use overwatch::services::{ServiceData, relay::OutboundRelay};
use tokio::sync::oneshot;

use crate::membership::{
    MembershipAdapter, MembershipAdapterError, PeerMultiaddrStream, SubnetworkPeers,
};

pub struct MembershipServiceAdapter<Backend, SdpAdapter, StorageAdapter, RuntimeServiceId>
where
    Backend: nomos_membership_service::backends::MembershipBackend,
    Backend::Settings: Clone,
    SdpAdapter: nomos_membership_service::adapters::sdp::SdpAdapter,
{
    relay: OutboundRelay<
        <MembershipService<Backend, SdpAdapter, StorageAdapter, RuntimeServiceId> as ServiceData>::Message,
    >,
    phantom: PhantomData<(Backend, SdpAdapter, RuntimeServiceId)>,
}

impl<Backend, SdpAdapter, StorageAdapter, RuntimeServiceId>
    MembershipServiceAdapter<Backend, SdpAdapter, StorageAdapter, RuntimeServiceId>
where
    SdpAdapter: nomos_membership_service::adapters::sdp::SdpAdapter + Send + Sync + 'static,
    Backend: nomos_membership_service::backends::MembershipBackend + Send + Sync + 'static,
    Backend::Settings: Clone,
    RuntimeServiceId: Send + Sync + 'static,
{
    async fn subscribe_stream(
        &self,
        service_type: ServiceType,
    ) -> Result<MembershipSnapshotStream, MembershipAdapterError> {
        let (sender, receiver) = oneshot::channel();

        self.relay
            .send(MembershipMessage::Subscribe {
                result_sender: sender,
                service_type,
            })
            .await
            .map_err(|(e, _)| MembershipAdapterError::Other(e.into()))?;

        receiver
            .await
            .map_err(|e| MembershipAdapterError::Other(e.into()))?
            .map_err(MembershipAdapterError::Backend)
    }
}

#[async_trait::async_trait]
impl<Backend, SdpAdapter, StorageAdapter, RuntimeServiceId> MembershipAdapter
    for MembershipServiceAdapter<Backend, SdpAdapter, StorageAdapter, RuntimeServiceId>
where
    SdpAdapter: nomos_membership_service::adapters::sdp::SdpAdapter + Send + Sync + 'static,
    Backend: nomos_membership_service::backends::MembershipBackend + Send + Sync + 'static,
    Backend::Settings: Clone,
    RuntimeServiceId: Send + Sync + 'static,
{
    type MembershipService =
        MembershipService<Backend, SdpAdapter, StorageAdapter, RuntimeServiceId>;
    type Id = PeerId;

    fn new(relay: OutboundRelay<<Self::MembershipService as ServiceData>::Message>) -> Self {
        Self {
            phantom: PhantomData,
            relay,
        }
    }

    async fn subscribe(&self) -> Result<PeerMultiaddrStream<Self::Id>, MembershipAdapterError> {
        let input_stream = self.subscribe_stream(ServiceType::DataAvailability).await?;
        let converted_stream = input_stream.map(|(session_id, providers_map)| {
            let mut peers_map: HashMap<PeerId, Multiaddr> = HashMap::new();
            let mut provider_mappings: HashMap<PeerId, ProviderId> = HashMap::new();

            for (provider_id, locators) in providers_map {
                // TODO: Support multiple multiaddrs in the membership.
                let Some(locator) = locators.first() else {
                    continue;
                };

                match peer_id_from_provider_id(provider_id.0.as_bytes()) {
                    Ok(peer_id) => {
                        peers_map.insert(peer_id, locator.0.clone());
                        provider_mappings.insert(peer_id, provider_id);
                    }
                    Err(err) => {
                        tracing::warn!(
                            "Failed to parse PeerId from provider_id: {:?}, error: {:?}",
                            provider_id.0,
                            err
                        );
                    }
                }
            }

            SubnetworkPeers {
                session_id,
                peers: peers_map,
                provider_mappings,
            }
        });
        Ok(Box::pin(converted_stream))
    }
}

pub fn peer_id_from_provider_id(pk_raw: &[u8]) -> Result<PeerId, DecodingError> {
    let ed_pub = ed25519::PublicKey::try_from_bytes(pk_raw)?;
    Ok(PeerId::from_public_key(&ed_pub.into()))
}
