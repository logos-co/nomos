pub mod backends;
pub mod membership;
pub mod storage;

use std::{
    collections::HashMap,
    fmt::{self, Debug, Display},
    marker::PhantomData,
    pin::Pin,
};

use async_trait::async_trait;
use backends::NetworkBackend;
use futures::Stream;
use libp2p::Multiaddr;
use nomos_core::block::BlockNumber;
use overwatch::{
    services::{
        state::{NoOperator, ServiceState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use serde::{Deserialize, Serialize};
use storage::{MembershipStorage, MembershipStorageAdapter};
use subnetworks_assignations::{MembershipCreator, MembershipHandler, SubnetworkAssignations};
use tokio::sync::oneshot;
use tokio_stream::StreamExt as _;

use crate::membership::{handler::DaMembershipHandler, MembershipAdapter};

pub enum DaNetworkMsg<Backend, Membership, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId>,
    Membership: MembershipHandler,
{
    Process(Backend::Message),
    Subscribe {
        kind: Backend::EventKind,
        sender: oneshot::Sender<Pin<Box<dyn Stream<Item = Backend::NetworkEvent> + Send>>>,
    },

    SubnetworksAtBlock {
        block_number: BlockNumber,
        sender: oneshot::Sender<SubnetworkAssignations<Membership::NetworkId, Membership::Id>>,
    },
}

impl<Backend, Membership, RuntimeServiceId> Debug
    for DaNetworkMsg<Backend, Membership, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId>,
    Membership: MembershipHandler,
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Process(msg) => write!(fmt, "DaNetworkMsg::Process({msg:?})"),
            Self::Subscribe { kind, .. } => {
                write!(fmt, "DaNetworkMsg::Subscribe{{ kind: {kind:?}}}")
            }
            Self::SubnetworksAtBlock { block_number, .. } => {
                write!(
                    fmt,
                    "DaNetworkMsg::SubnetworksAtBlock{{ block_number: {block_number} }}"
                )
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct NetworkConfig<Backend: NetworkBackend<RuntimeServiceId>, Membership, RuntimeServiceId> {
    pub backend: Backend::Settings,
    pub membership: Membership,
}

impl<Backend: NetworkBackend<RuntimeServiceId>, Membership, RuntimeServiceId> Debug
    for NetworkConfig<Backend, Membership, RuntimeServiceId>
where
    Membership: Clone + Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "NetworkConfig {{ backend: {:?}}}", self.backend)
    }
}

pub struct NetworkService<
    Backend,
    Membership,
    MembershipServiceAdapter,
    StorageAdapter,
    RuntimeServiceId,
> where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Membership: MembershipHandler,
{
    backend: Backend,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    membership: DaMembershipHandler<Membership>,
    phantom: PhantomData<MembershipServiceAdapter>,
}

pub struct NetworkState<
    Backend,
    Membership,
    MembershipServiceAdapter,
    StorageAdapter,
    RuntimeServiceId,
> where
    Backend: NetworkBackend<RuntimeServiceId>,
{
    backend: Backend::State,
    phantom: PhantomData<(Membership, MembershipServiceAdapter, StorageAdapter)>,
}

impl<Backend, Membership, MembershipServiceAdapter, StorageAdapter, RuntimeServiceId> ServiceData
    for NetworkService<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        RuntimeServiceId,
    >
where
    Backend: NetworkBackend<RuntimeServiceId> + 'static + Send,
    Membership: MembershipHandler,
{
    type Settings = NetworkConfig<Backend, Membership, RuntimeServiceId>;
    type State = NetworkState<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        RuntimeServiceId,
    >;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaNetworkMsg<Backend, Membership, RuntimeServiceId>;
}

#[async_trait]
impl<Backend, Membership, MembershipServiceAdapter, StorageAdapter, RuntimeServiceId>
    ServiceCore<RuntimeServiceId>
    for NetworkService<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        RuntimeServiceId,
    >
where
    Backend: NetworkBackend<RuntimeServiceId, Membership = DaMembershipHandler<Membership>>
        + Send
        + 'static,
    Backend::State: Send + Sync,
    Membership: MembershipCreator + Clone + Send + Sync + 'static,
    Membership::Id: Send + Sync,
    Membership::NetworkId: Send,
    MembershipServiceAdapter: MembershipAdapter<Id = Membership::Id> + Send + Sync + 'static,
    StorageAdapter: MembershipStorageAdapter<
            <Membership as MembershipHandler>::Id,
            <Membership as MembershipHandler>::NetworkId,
        > + Default
        + Send
        + Sync
        + 'static,
    <MembershipServiceAdapter::MembershipService as ServiceData>::Message: Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self>
        + Clone
        + Display
        + Send
        + Sync
        + Debug
        + AsServiceId<MembershipServiceAdapter::MembershipService>,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let membership = DaMembershipHandler::new(settings.membership);

        Ok(Self {
            backend: <Backend as NetworkBackend<RuntimeServiceId>>::new(
                settings.backend,
                service_resources_handle.overwatch_handle.clone(),
                membership.clone(),
            ),
            service_resources_handle,
            membership,
            phantom: PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            service_resources_handle:
                OpaqueServiceResourcesHandle::<Self, RuntimeServiceId> {
                    ref mut inbound_relay,
                    ref status_updater,
                    ref overwatch_handle,
                    ..
                },
            ref mut backend,
            ref membership,
            ..
        } = self;

        let membership_service_relay = overwatch_handle
            .relay::<MembershipServiceAdapter::MembershipService>()
            .await?;

        let storage_adapter = StorageAdapter::default();
        let membership_storage = MembershipStorage::new(storage_adapter, membership.clone());

        let membership_service_adapter = MembershipServiceAdapter::new(membership_service_relay);

        let mut stream = membership_service_adapter.subscribe().await.map_err(|e| {
            tracing::error!("Failed to subscribe to membership service: {e}");
            e
        })?;

        status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        loop {
            tokio::select! {
                Some(msg) = inbound_relay.recv() => {
                    Self::handle_network_service_message(msg, backend, &membership_storage).await;
                }
                Some((block_number, providers)) = stream.next() => {
                    tracing::debug!(
                        "Received membership update for block {}: {:?}",
                        block_number, providers
                    );
                    Self::handle_membership_update(block_number, providers, &membership_storage).await;
                }
            }
        }
    }
}

impl<Backend, Membership, MembershipServiceAdapter, StorageAdapter, RuntimeServiceId> Drop
    for NetworkService<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        RuntimeServiceId,
    >
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Membership: MembershipHandler,
{
    fn drop(&mut self) {
        self.backend.shutdown();
    }
}

impl<Backend, Membership, MembershipServiceAdapter, StorageAdapter, RuntimeServiceId>
    NetworkService<Backend, Membership, MembershipServiceAdapter, StorageAdapter, RuntimeServiceId>
where
    StorageAdapter: MembershipStorageAdapter<
            <Membership as MembershipHandler>::Id,
            <Membership as MembershipHandler>::NetworkId,
        > + Send
        + Sync,

    Membership::Id: Send + Sync,
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Backend::State: Send + Sync,
    Membership: MembershipCreator + Clone + Send + Sync + 'static,
{
    async fn handle_network_service_message(
        msg: DaNetworkMsg<Backend, Membership, RuntimeServiceId>,
        backend: &mut Backend,
        membership_storage: &MembershipStorage<StorageAdapter, Membership>,
    ) {
        match msg {
            DaNetworkMsg::Process(msg) => {
                // split sending in two steps to help the compiler understand we do not
                // need to hold an instance of &I (which is not Send) across an await point
                let send = backend.process(msg);
                send.await;
            }
            DaNetworkMsg::Subscribe { kind, sender } => sender
                .send(backend.subscribe(kind).await)
                .unwrap_or_else(|_| {
                    tracing::warn!(
                        "client hung up before a subscription handle could be established"
                    );
                }),
            DaNetworkMsg::SubnetworksAtBlock {
                block_number,
                sender,
            } => {
                if let Some(membership) = membership_storage
                    .get_historic_membership(block_number)
                    .await
                    .unwrap_or_else(|e| {
                        tracing::error!(
                            "Failed to get historic membership for block {block_number}: {e}"
                        );
                        None
                    })
                {
                    let assignations = membership.subnetworks();
                    sender.send(assignations).unwrap_or_else(|_| {
                        tracing::warn!(
                            "client hung up before a subnetwork assignations handle could be established"
                        );
                    });
                } else {
                    tracing::warn!("No membership found for block number {block_number}");
                    sender.send(SubnetworkAssignations::default()).unwrap_or_else(|_| {
                        tracing::warn!(
                            "client hung up before a subnetwork assignations handle could be established"
                        );
                    });
                }
            }
        }
    }

    async fn handle_membership_update(
        block_number: BlockNumber,
        update: HashMap<<Membership as MembershipHandler>::Id, Multiaddr>,
        storage: &MembershipStorage<StorageAdapter, Membership>,
    ) {
        storage
            .update(block_number, update)
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Failed to update membership at block {block_number}: {e}");
            });
    }
}

impl<Backend: NetworkBackend<RuntimeServiceId>, Membership, RuntimeServiceId> Clone
    for NetworkConfig<Backend, Membership, RuntimeServiceId>
where
    Membership: Clone,
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            membership: self.membership.clone(),
        }
    }
}

impl<
        Backend: NetworkBackend<RuntimeServiceId>,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        RuntimeServiceId,
    > Clone
    for NetworkState<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        RuntimeServiceId,
    >
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            phantom: PhantomData,
        }
    }
}

impl<
        Backend: NetworkBackend<RuntimeServiceId>,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        RuntimeServiceId,
    > ServiceState
    for NetworkState<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        RuntimeServiceId,
    >
where
    Membership: Clone,
{
    type Settings = NetworkConfig<Backend, Membership, RuntimeServiceId>;
    type Error = <Backend::State as ServiceState>::Error;

    fn from_settings(settings: &Self::Settings) -> Result<Self, Self::Error> {
        Backend::State::from_settings(&settings.backend).map(|backend| Self {
            backend,
            phantom: PhantomData,
        })
    }
}
