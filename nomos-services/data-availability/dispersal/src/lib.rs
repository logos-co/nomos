use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
};

use nomos_core::da::blob::metadata;
use nomos_da_network_core::{PeerId, SubnetworkId};
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
use serde::{Deserialize, Serialize};
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
pub enum DaDispersalMsg<Metadata> {
    Disperse {
        data: Vec<u8>,
        metadata: Metadata,
        reply_channel: oneshot::Sender<Result<(), DynError>>,
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
    Backend::Settings: Clone,
    NetworkAdapter: DispersalNetworkAdapter,
    MempoolAdapter: DaMempoolAdapter,
    Metadata: metadata::Metadata + Debug + 'static,
{
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
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
    Backend::Settings: Clone,
    NetworkAdapter: DispersalNetworkAdapter,
    MempoolAdapter: DaMempoolAdapter,
    Metadata: metadata::Metadata + Debug + 'static,
{
    type Settings = DispersalServiceSettings<Backend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaDispersalMsg<Metadata>;
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
    NetworkAdapter: DispersalNetworkAdapter<SubnetworkId = Membership::NetworkId> + Send,
    <NetworkAdapter::NetworkService as ServiceData>::Message: 'static,
    MempoolAdapter: DaMempoolAdapter,
    <MempoolAdapter::MempoolService as ServiceData>::Message: 'static,
    Metadata: metadata::Metadata + Debug + Send + 'static,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + Send
        + AsServiceId<NetworkAdapter::NetworkService>
        + AsServiceId<MempoolAdapter::MempoolService>,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self {
            service_state,
            _backend: PhantomData,
        })
    }

    async fn run(self) -> Result<(), DynError> {
        let Self { service_state, .. } = self;

        let DispersalServiceSettings {
            backend: backend_settings,
        } = service_state.settings_reader.get_updated_settings();
        let network_relay = service_state
            .overwatch_handle
            .relay::<NetworkAdapter::NetworkService>()
            .await?;
        let network_adapter = NetworkAdapter::new(network_relay);
        let mempool_relay = service_state
            .overwatch_handle
            .relay::<MempoolAdapter::MempoolService>()
            .await?;
        let mempool_adapter = MempoolAdapter::new(mempool_relay);
        let backend = Backend::init(backend_settings, network_adapter, mempool_adapter);
        let mut inbound_relay = service_state.inbound_relay;
        while let Some(dispersal_msg) = inbound_relay.recv().await {
            match dispersal_msg {
                DaDispersalMsg::Disperse {
                    data,
                    metadata,
                    reply_channel,
                } => {
                    if let Err(Err(e)) =
                        reply_channel.send(backend.process_dispersal(data, metadata).await)
                    {
                        error!("Error forwarding dispersal response: {e}");
                    }
                }
            }
        }

        Ok(())
    }
}
