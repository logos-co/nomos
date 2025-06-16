use std::{collections::HashMap, marker::PhantomData, pin::Pin};

use futures::{stream, StreamExt};
use libp2p::{
    identity::{self, ed25519, PublicKey},
    Multiaddr, PeerId,
};
use nomos_da_network_core::SubnetworkId;
use nomos_membership::{
    MembershipMessage, MembershipProviders, MembershipService, MembershipSnapshotStream,
};
use nomos_sdp_core::{ProviderId, ServiceType};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use subnetworks_assignations::MembershipCreator;
use tokio::sync::oneshot;

use crate::{
    membership::{
        handler::DaMembershipHandler, MembershipAdapter, MembershipAdapterError,
        PeerMultiaddrStream,
    },
    storage::MembershipStorage,
};

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
                    locators.first().map(|locator| {
                        let peer_id = PeerId::random(); //provider_to_peer_id(provider_id)?;
                        let multiaddr = locator.0.clone();
                        (peer_id, multiaddr)
                    })
                })
                .collect();

            (block_number, peers_map)
        });

        Ok(Box::pin(converted_stream))
    }
}
