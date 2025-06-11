use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    time::Duration,
};

use nomos_core::da::blob::metadata;
use nomos_da_network_core::{PeerId, SubnetworkId};
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use serde::{Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use subnetworks_assignations::MembershipHandler;
use tokio::sync::oneshot;
use tracing::error;

use crate::{
    adapters::{mempool::DaMempoolAdapter, network::DispersalNetworkAdapter},
    backend::DispersalBackend,
};

pub mod adapters;
pub mod backend;

#[derive(Debug)]
pub enum DaDispersalMsg<Metadata, B: DispersalBackend> {
    Disperse {
        data: Vec<u8>,
        metadata: Metadata,
        reply_channel: oneshot::Sender<Result<B::BlobId, DynError>>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DispersalServiceSettings<BackendSettings> {
    pub backend: BackendSettings,
}

pub struct DispersalService<
    Backend,
    NetworkAdapter,
    MempoolAdapter,
    Membership,
    Metadata,
    RuntimeServiceId,
> where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    Backend: DispersalBackend<NetworkAdapter = NetworkAdapter, Metadata = Metadata>,
    Backend::BlobId: Serialize,
    Backend::Settings: Clone,
    NetworkAdapter: DispersalNetworkAdapter,
    MempoolAdapter: DaMempoolAdapter,
    Metadata: metadata::Metadata + Debug + 'static,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _backend: PhantomData<Backend>,
}

impl<Backend, NetworkAdapter, MempoolAdapter, Membership, Metadata, RuntimeServiceId> ServiceData
    for DispersalService<
        Backend,
        NetworkAdapter,
        MempoolAdapter,
        Membership,
        Metadata,
        RuntimeServiceId,
    >
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    Backend: DispersalBackend<NetworkAdapter = NetworkAdapter, Metadata = Metadata>,
    Backend::BlobId: Serialize,
    Backend::Settings: Clone,
    NetworkAdapter: DispersalNetworkAdapter,
    MempoolAdapter: DaMempoolAdapter,
    Metadata: metadata::Metadata + Debug + 'static,
{
    type Settings = DispersalServiceSettings<Backend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaDispersalMsg<Metadata, Backend>;
}

#[async_trait::async_trait]
impl<Backend, NetworkAdapter, MempoolAdapter, Membership, Metadata, RuntimeServiceId>
    ServiceCore<RuntimeServiceId>
    for DispersalService<
        Backend,
        NetworkAdapter,
        MempoolAdapter,
        Membership,
        Metadata,
        RuntimeServiceId,
    >
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    Backend: DispersalBackend<
            NetworkAdapter = NetworkAdapter,
            MempoolAdapter = MempoolAdapter,
            Metadata = Metadata,
        > + Send
        + Sync,
    Backend::Settings: Clone + Send + Sync,
    Backend::BlobId: Serialize,
    NetworkAdapter: DispersalNetworkAdapter<SubnetworkId = Membership::NetworkId> + Send,
    <NetworkAdapter::NetworkService as ServiceData>::Message: 'static,
    MempoolAdapter: DaMempoolAdapter,
    <MempoolAdapter::MempoolService as ServiceData>::Message: 'static,
    Metadata: metadata::Metadata + Debug + Send + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + Send
        + AsServiceId<Self>
        + AsServiceId<NetworkAdapter::NetworkService>
        + AsServiceId<MempoolAdapter::MempoolService>
        + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self {
            service_resources_handle,
            _backend: PhantomData,
        })
    }

    async fn run(self) -> Result<(), DynError> {
        let Self {
            service_resources_handle,
            ..
        } = self;

        let DispersalServiceSettings {
            backend: backend_settings,
        } = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        let network_relay = service_resources_handle
            .overwatch_handle
            .relay::<NetworkAdapter::NetworkService>()
            .await?;
        let network_adapter = NetworkAdapter::new(network_relay);
        let mempool_relay = service_resources_handle
            .overwatch_handle
            .relay::<MempoolAdapter::MempoolService>()
            .await?;
        let mempool_adapter = MempoolAdapter::new(mempool_relay);
        let backend = Backend::init(backend_settings, network_adapter, mempool_adapter);
        let mut inbound_relay = service_resources_handle.inbound_relay;

        service_resources_handle.status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkAdapter::NetworkService,
            MempoolAdapter::MempoolService
        )
        .await?;

        while let Some(dispersal_msg) = inbound_relay.recv().await {
            match dispersal_msg {
                DaDispersalMsg::Disperse {
                    data,
                    metadata,
                    reply_channel,
                } => {
                    let response = backend.process_dispersal(data, metadata).await;
                    if let Err(Err(e)) = reply_channel.send(response) {
                        error!("Error forwarding dispersal response: {e}");
                    }
                }
            }
        }

        Ok(())
    }
}
