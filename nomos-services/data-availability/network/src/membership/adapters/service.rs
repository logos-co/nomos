use std::{collections::HashMap, marker::PhantomData};

use futures::StreamExt as _;
use libp2p::{Multiaddr, PeerId, core::signed_envelope::DecodingError};
use nomos_core::sdp::ServiceType;
use nomos_libp2p::ed25519;
use nomos_membership_service::{MembershipMessage, MembershipService, MembershipSnapshotStream};
use overwatch::services::{ServiceData, relay::OutboundRelay};
use tokio::sync::oneshot;

use crate::membership::{MembershipAdapter, MembershipAdapterError, PeerMultiaddrStream};

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
            let peers_map: HashMap<PeerId, Multiaddr> = providers_map
                .into_iter()
                .filter_map(|(provider_id, locators)| {
                    // TODO: Support multiple multiaddrs in the membership.
                    let locator = locators.first()?;
                    match peer_id_from_provider_id(provider_id.0.as_bytes()) {
                        Ok(peer_id) => Some((peer_id, locator.0.clone())),
                        Err(err) => {
                            tracing::warn!(
                                "Failed to parse PeerId from provider_id: {:?}, error: {:?}",
                                provider_id.0,
                                err
                            );
                            None
                        }
                    }
                })
                .collect();

            (session_id, peers_map)
        });

        Ok(Box::pin(converted_stream))
    }
}

pub fn peer_id_from_provider_id(pk_raw: &[u8]) -> Result<PeerId, DecodingError> {
    let ed_pub = ed25519::PublicKey::try_from_bytes(pk_raw)?;
    Ok(PeerId::from_public_key(&ed_pub.into()))
}
