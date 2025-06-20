use std::{collections::HashMap, marker::PhantomData};

use futures::StreamExt as _;
use libp2p::{core::signed_envelope::DecodingError, Multiaddr, PeerId};
use nomos_libp2p::ed25519;
use nomos_membership::{MembershipMessage, MembershipService, MembershipSnapshotStream};
use nomos_sdp_core::ServiceType;
use overwatch::services::{relay::OutboundRelay, ServiceData};
use tokio::sync::oneshot;

use crate::membership::{MembershipAdapter, MembershipAdapterError, PeerMultiaddrStream};

pub struct MembershipServiceAdapter<Backend, SdpAdapter, RuntimeServiceId>
where
    Backend: nomos_membership::backends::MembershipBackend,
    Backend::Settings: Clone,
    SdpAdapter: nomos_membership::adapters::SdpAdapter,
{
    relay: OutboundRelay<
        <MembershipService<Backend, SdpAdapter, RuntimeServiceId> as ServiceData>::Message,
    >,
    phantom: PhantomData<(Backend, SdpAdapter, RuntimeServiceId)>,
}

impl<Backend, SdpAdapter, RuntimeServiceId>
    MembershipServiceAdapter<Backend, SdpAdapter, RuntimeServiceId>
where
    SdpAdapter: nomos_membership::adapters::SdpAdapter + Send + Sync + 'static,
    Backend: nomos_membership::backends::MembershipBackend + Send + Sync + 'static,
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

        let res = receiver
            .await
            .map_err(|e| MembershipAdapterError::Other(e.into()))?
            .map_err(MembershipAdapterError::Backend);

        res
    }
}

#[async_trait::async_trait]
impl<Backend, SdpAdapter, RuntimeServiceId> MembershipAdapter
    for MembershipServiceAdapter<Backend, SdpAdapter, RuntimeServiceId>
where
    SdpAdapter: nomos_membership::adapters::SdpAdapter + Send + Sync + 'static,
    Backend: nomos_membership::backends::MembershipBackend + Send + Sync + 'static,
    Backend::Settings: Clone,
    RuntimeServiceId: Send + Sync + 'static,
{
    type MembershipService = MembershipService<Backend, SdpAdapter, RuntimeServiceId>;
    type Id = PeerId;

    fn new(relay: OutboundRelay<<Self::MembershipService as ServiceData>::Message>) -> Self {
        Self {
            phantom: PhantomData,
            relay,
        }
    }

    async fn subscribe(&self) -> Result<PeerMultiaddrStream<Self::Id>, MembershipAdapterError> {
        let input_stream = self.subscribe_stream(ServiceType::DataAvailability).await?;

        let converted_stream = input_stream.map(|(block_number, providers_map)| {
            let peers_map: HashMap<PeerId, Multiaddr> = providers_map
                .into_iter()
                .filter_map(|(provider_id, locators)| {
                    // TODO: Support multiple multiaddrs in the membership.
                    let locator = locators.first()?;
                    match peer_id_from_provider_id(&provider_id.0) {
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

            (block_number, peers_map)
        });

        Ok(Box::pin(converted_stream))
    }
}

pub fn peer_id_from_provider_id(pk_raw: &[u8]) -> Result<PeerId, DecodingError> {
    let ed_pub = ed25519::PublicKey::try_from_bytes(pk_raw)?;
    Ok(PeerId::from_public_key(&ed_pub.into()))
}
