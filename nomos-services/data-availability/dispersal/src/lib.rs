use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    time::Duration,
};

use adapters::wallet::{DaWalletAdapter, mock::MockWalletAdapter};
use backend::DispersalTask;
use futures::{StreamExt as _, stream::FuturesUnordered};
use nomos_core::mantle::{
    Transaction as _,
    ops::channel::{ChannelId, Ed25519PublicKey, MsgId},
};
use nomos_da_network_core::{PeerId, SubnetworkId};
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        state::{NoOperator, NoState},
    },
};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use services_utils::wait_until_services_are_ready;
use subnetworks_assignations::MembershipHandler;
use tokio::sync::oneshot;
use tracing::error;

use crate::{adapters::network::DispersalNetworkAdapter, backend::DispersalBackend};

pub mod adapters;
pub mod backend;

#[derive(Debug)]
pub enum DaDispersalMsg<B: DispersalBackend> {
    Disperse {
        channel_id: ChannelId,
        parent_msg_id: MsgId,
        signer: Ed25519PublicKey,
        data: Vec<u8>,
        reply_channel: oneshot::Sender<Result<B::BlobId, DynError>>,
    },
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DispersalServiceSettings<BackendSettings> {
    pub backend: BackendSettings,
}

pub type DispersalService<Backend, NetworkAdapter, Membership, RuntimeServiceId> =
    GenericDispersalService<
        Backend,
        NetworkAdapter,
        MockWalletAdapter,
        Membership,
        RuntimeServiceId,
    >;

pub struct GenericDispersalService<
    Backend,
    NetworkAdapter,
    WalletAdapter,
    Membership,
    RuntimeServiceId,
> where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    Backend: DispersalBackend<NetworkAdapter = NetworkAdapter>,
    Backend::BlobId: Serialize,
    Backend::Settings: Clone,
    NetworkAdapter: DispersalNetworkAdapter,
    WalletAdapter: DaWalletAdapter,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _backend: PhantomData<Backend>,
}

impl<Backend, NetworkAdapter, WalletAdapter, Membership, RuntimeServiceId> ServiceData
    for GenericDispersalService<
        Backend,
        NetworkAdapter,
        WalletAdapter,
        Membership,
        RuntimeServiceId,
    >
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    Backend: DispersalBackend<NetworkAdapter = NetworkAdapter>,
    Backend::BlobId: Serialize,
    Backend::Settings: Clone,
    NetworkAdapter: DispersalNetworkAdapter,
    WalletAdapter: DaWalletAdapter,
{
    type Settings = DispersalServiceSettings<Backend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaDispersalMsg<Backend>;
}

#[async_trait::async_trait]
impl<Backend, NetworkAdapter, WalletAdapter, Membership, RuntimeServiceId>
    ServiceCore<RuntimeServiceId>
    for GenericDispersalService<
        Backend,
        NetworkAdapter,
        WalletAdapter,
        Membership,
        RuntimeServiceId,
    >
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
    Backend: DispersalBackend<NetworkAdapter = NetworkAdapter, WalletAdapter = WalletAdapter>
        + Send
        + Sync,
    Backend::Settings: Clone + Send + Sync,
    Backend::BlobId: Debug + Serialize,
    NetworkAdapter: DispersalNetworkAdapter<SubnetworkId = Membership::NetworkId> + Send,
    <NetworkAdapter::NetworkService as ServiceData>::Message: 'static,
    WalletAdapter: DaWalletAdapter + Send,
    RuntimeServiceId: Debug
        + Sync
        + Display
        + Send
        + AsServiceId<Self>
        + AsServiceId<NetworkAdapter::NetworkService>
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
        let wallet_adapter = WalletAdapter::new();
        let backend = Backend::init(backend_settings, network_adapter, wallet_adapter);
        let mut inbound_relay = service_resources_handle.inbound_relay;
        let mut disperse_tasks: FuturesUnordered<DispersalTask> = FuturesUnordered::new();

        service_resources_handle.status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        wait_until_services_are_ready!(
            &service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            NetworkAdapter::NetworkService
        )
        .await?;

        loop {
            tokio::select! {
                Some(dispersal_msg) = inbound_relay.recv() => {
                    let DaDispersalMsg::Disperse {
                        channel_id,
                        parent_msg_id,
                        signer,
                        data,
                        reply_channel,
                    } = dispersal_msg;
                    match backend.process_dispersal(
                        channel_id,
                        parent_msg_id,
                        signer,
                        data,
                        reply_channel,
                    )
                    .await {
                        Ok(task) => disperse_tasks.push(task),
                        Err(e) => error!("Error while processing dispersal: {e}"),
                    }
                }
                Some(dispersal_result) = disperse_tasks.next() => {
                    if let (channel_id, Some(tx)) = dispersal_result {
                        tracing::info!("Dispersal retry successful for channel: {channel_id:?}, tx: {:?}", tx.hash());
                    } else {
                        tracing::error!("Dispersal failed after all retry attempts");
                    }
                }
            }
        }
    }
}
