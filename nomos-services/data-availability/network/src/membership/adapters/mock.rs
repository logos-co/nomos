use std::{marker::PhantomData, sync::Arc};

use libp2p::PeerId;
use nomos_da_network_core::SubnetworkId;
use nomos_membership::{MembershipMessage, MembershipService, MembershipSnapshotStream};
use nomos_sdp_core::ServiceType;
use overwatch::services::{relay::OutboundRelay, ServiceData};
use subnetworks_assignations::MembershipCreator;
use tokio::sync::oneshot;
use tokio_stream::StreamExt as _;

use crate::{
    membership::{
        adapters::{MembershipAdapter, MembershipAdapterError},
        handler::DaMembershipHandler,
    },
    storage::MembershipStorage,
    MembershipProviders,
};

pub struct MockMembershipAdapter<Backend, Membership, SdpAdapter, Storage, RuntimeServiceId>
where
    Membership: MembershipCreator + Clone,
    Storage: MembershipStorage,
    Backend: nomos_membership::backends::MembershipBackend,
    Backend::Settings: Clone,
    SdpAdapter: nomos_membership::adapters::SdpAdapter,
{
    handler: DaMembershipHandler<Membership>,
    storage: Storage,
    relay: OutboundRelay<
        <MembershipService<Backend, SdpAdapter, RuntimeServiceId> as ServiceData>::Message,
    >,
    phantom: PhantomData<(Backend, SdpAdapter, RuntimeServiceId)>,
}

#[async_trait::async_trait]
impl<Backend, Membership, SdpAdapter, Storage, RuntimeServiceId>
    MembershipAdapter<Membership, Storage>
    for MockMembershipAdapter<Backend, Membership, SdpAdapter, Storage, RuntimeServiceId>
where
    Membership:
        MembershipCreator<NetworkId = SubnetworkId, Id = PeerId> + Clone + Send + Sync + 'static,
    Storage: MembershipStorage + Send + Sync + 'static,
    SdpAdapter: nomos_membership::adapters::SdpAdapter + Send + Sync + 'static,
    Backend: nomos_membership::backends::MembershipBackend + Send + Sync + 'static,
    Backend::Settings: Clone,
    RuntimeServiceId: Send + Sync + 'static,
{
    type MembershipService = MembershipService<Backend, SdpAdapter, RuntimeServiceId>;
    fn new(
        relay: OutboundRelay<<Self::MembershipService as ServiceData>::Message>,
        handler: DaMembershipHandler<Membership>,
        storage: Storage,
    ) -> Self {
        Self {
            handler,
            storage,
            phantom: PhantomData,
            relay,
        }
    }

    async fn get_historic_membership(&self, block_number: u64) -> Option<Membership> {
        let assignations = self.storage.get(block_number)?;
        Some(self.handler.membership().init(assignations))
    }

    async fn bootstrap(self: Arc<Self>) -> Result<(), MembershipAdapterError> {
        let mut stream = self.subscribe_stream(ServiceType::DataAvailability).await?;

        tokio::spawn(async move {
            while let Some((block_number, new_members)) = stream.next().await {
                Self::update(block_number, new_members);
            }
        });

        Ok(())
    }
}

impl<Backend, Membership, SdpAdapter, Storage, RuntimeServiceId>
    MockMembershipAdapter<Backend, Membership, SdpAdapter, Storage, RuntimeServiceId>
where
    Membership: MembershipCreator<NetworkId = SubnetworkId, Id = PeerId> + Clone + Send + Sync,
    Storage: MembershipStorage + Send + Sync,
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

    fn update(_block_number: u64, _new_members: MembershipProviders) {
        // todo: transpose MembershipProviders to peer_id -> multiaddr mapping
        // membership service instead of config
        // todo: implement update with loading initial state from

        // let updated_membership =
        // self.handler.membership().update(new_members);
        // let assignations = updated_membership.subnetworks();

        // self.handler.update(updated_membership);
        // self.storage.store(block_number, assignations);
    }
}
