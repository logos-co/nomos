pub mod addressbook;
pub mod api;
pub mod backends;
pub mod membership;
pub mod storage;

use std::{
    collections::{HashMap, HashSet},
    fmt::{self, Debug, Display},
    marker::PhantomData,
    pin::Pin,
};

use async_trait::async_trait;
use backends::NetworkBackend;
use futures::Stream;
use kzgrs_backend::common::share::{DaShare, DaSharesCommitments};
use libp2p::{Multiaddr, PeerId};
use nomos_core::{block::BlockNumber, da::BlobId, header::HeaderId};
use nomos_da_network_core::{addressbook::AddressBookHandler as _, SubnetworkId};
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

use crate::{
    addressbook::{AddressBook, AddressBookSnapshot},
    api::ApiAdapter as ApiAdapterTrait,
    membership::{handler::DaMembershipHandler, MembershipAdapter},
};

pub type DaAddressbook = AddressBook;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MembershipResponse {
    pub assignations: SubnetworkAssignations<SubnetworkId, PeerId>,
    pub addressbook: AddressBookSnapshot<PeerId>,
}

pub enum DaNetworkMsg<Backend, Commitments, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId>,
{
    Process(Backend::Message),
    Subscribe {
        kind: Backend::EventKind,
        sender: oneshot::Sender<Pin<Box<dyn Stream<Item = Backend::NetworkEvent> + Send>>>,
    },
    GetMembership {
        block_number: BlockNumber,
        sender: oneshot::Sender<MembershipResponse>,
    },
    GetCommitments {
        blob_id: BlobId,
        sender: oneshot::Sender<Option<Commitments>>,
    },
    RequestHistoricSample {
        block_number: BlockNumber,
        block_id: HeaderId,
        blob_ids: HashSet<BlobId>,
    },
}

impl<Backend, Commitments, RuntimeServiceId> Debug
    for DaNetworkMsg<Backend, Commitments, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId>,
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Process(msg) => write!(fmt, "DaNetworkMsg::Process({msg:?})"),
            Self::Subscribe { kind, .. } => {
                write!(fmt, "DaNetworkMsg::Subscribe{{ kind: {kind:?}}}")
            }
            Self::GetMembership { block_number, .. } => {
                write!(
                    fmt,
                    "DaNetworkMsg::SubnetworksAtBlock{{ block_number: {block_number} }}"
                )
            }
            Self::GetCommitments { blob_id, .. } => {
                write!(
                    fmt,
                    "DaNetworkMsg::GetCommitments{{ blob_id: {blob_id:?} }}"
                )
            }
            Self::RequestHistoricSample {
                block_number,
                blob_ids,
                block_id,
            } => {
                write!(
                    fmt,
                    "DaNetworkMsg::RequestHistoricSample{{ block_number: {block_number}, blob_ids: {blob_ids:?}, block_id: {block_id} }}"
                )
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct NetworkConfig<
    Backend: NetworkBackend<RuntimeServiceId>,
    Membership,
    ApiAdapterSettings,
    RuntimeServiceId,
> {
    pub backend: Backend::Settings,
    pub membership: Membership,
    pub api_adapter_settings: ApiAdapterSettings,
}

impl<
        Backend: NetworkBackend<RuntimeServiceId>,
        Membership,
        ApiAdapterSettings,
        RuntimeServiceId,
    > Debug for NetworkConfig<Backend, Membership, ApiAdapterSettings, RuntimeServiceId>
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
    ApiAdapter,
    RuntimeServiceId,
> where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Membership: MembershipHandler,
    ApiAdapter: ApiAdapterTrait,
{
    backend: Backend,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    membership: DaMembershipHandler<Membership>,
    addressbook: DaAddressbook,
    api_adapter: ApiAdapter,
    phantom: PhantomData<MembershipServiceAdapter>,
}

pub struct NetworkState<
    Backend,
    Membership,
    MembershipServiceAdapter,
    StorageAdapter,
    ApiAdapter,
    RuntimeServiceId,
> where
    Backend: NetworkBackend<RuntimeServiceId>,
{
    backend: Backend::State,
    phantom: PhantomData<(
        Membership,
        MembershipServiceAdapter,
        ApiAdapter,
        StorageAdapter,
    )>,
}

impl<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
        RuntimeServiceId,
    > ServiceData
    for NetworkService<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    Backend: NetworkBackend<RuntimeServiceId> + 'static + Send,
    Membership: MembershipHandler,
    ApiAdapter: ApiAdapterTrait,
{
    type Settings = NetworkConfig<Backend, Membership, ApiAdapter::Settings, RuntimeServiceId>;
    type State = NetworkState<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
        RuntimeServiceId,
    >;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaNetworkMsg<Backend, DaSharesCommitments, RuntimeServiceId>;
}

#[async_trait]
impl<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
        RuntimeServiceId,
    > ServiceCore<RuntimeServiceId>
    for NetworkService<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    Backend: NetworkBackend<
            RuntimeServiceId,
            Membership = DaMembershipHandler<Membership>,
            HistoricMembership = Membership,
            Addressbook = DaAddressbook,
        > + Send
        + Sync
        + 'static,
    Backend::State: Send + Sync,
    Membership:
        MembershipCreator<Id = PeerId, NetworkId = SubnetworkId> + Clone + Send + Sync + 'static,
    Membership::Id: Send + Sync,
    Membership::NetworkId: Send,
    MembershipServiceAdapter: MembershipAdapter<Id = Membership::Id> + Send + Sync + 'static,
    StorageAdapter: MembershipStorageAdapter<PeerId, SubnetworkId> + Send + Sync + 'static,
    ApiAdapter: ApiAdapterTrait<
            Share = DaShare,
            BlobId = BlobId,
            Commitments = DaSharesCommitments,
            Membership = DaMembershipHandler<Membership>,
            Addressbook = DaAddressbook,
        > + Send
        + Sync
        + 'static,
    ApiAdapter::Settings: Clone + Send + Sync,
    <MembershipServiceAdapter::MembershipService as ServiceData>::Message: Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self>
        + Clone
        + Display
        + Send
        + Sync
        + Debug
        + AsServiceId<MembershipServiceAdapter::MembershipService>
        + AsServiceId<StorageAdapter::StorageService>,
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
        let addressbook = DaAddressbook::default();

        let api_adapter = ApiAdapter::new(
            settings.api_adapter_settings.clone(),
            membership.clone(),
            addressbook.clone(),
        );

        Ok(Self {
            backend: <Backend as NetworkBackend<RuntimeServiceId>>::new(
                settings.backend,
                service_resources_handle.overwatch_handle.clone(),
                membership.clone(),
                addressbook.clone(),
            ),
            service_resources_handle,
            membership,
            addressbook,
            api_adapter,
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
            ref api_adapter,
            ref addressbook,
            ..
        } = self;

        let membership_service_relay = overwatch_handle
            .relay::<MembershipServiceAdapter::MembershipService>()
            .await?;

        let storage_service_relay = overwatch_handle
            .relay::<StorageAdapter::StorageService>()
            .await?;

        let storage_adapter = StorageAdapter::new(storage_service_relay);
        let membership_storage =
            MembershipStorage::new(storage_adapter, membership.clone(), addressbook.clone());

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
                    Self::handle_network_service_message(msg, backend, &membership_storage, api_adapter, addressbook).await;
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

impl<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
        RuntimeServiceId,
    > Drop
    for NetworkService<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Membership: MembershipHandler,
    ApiAdapter: ApiAdapterTrait,
{
    fn drop(&mut self) {
        self.backend.shutdown();
    }
}

impl<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
        RuntimeServiceId,
    >
    NetworkService<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    StorageAdapter: MembershipStorageAdapter<
            <Membership as MembershipHandler>::Id,
            <Membership as MembershipHandler>::NetworkId,
        > + Send
        + Sync,
    ApiAdapter:
        ApiAdapterTrait<BlobId = BlobId, Commitments = DaSharesCommitments> + Send + Sync + 'static,
    ApiAdapter::Settings: Clone + Send,
    Backend:
        NetworkBackend<RuntimeServiceId, HistoricMembership = Membership> + Send + Sync + 'static,
    Backend::State: Send + Sync,
    Membership:
        MembershipCreator<Id = PeerId, NetworkId = SubnetworkId> + Clone + Send + Sync + 'static,
{
    async fn handle_network_service_message(
        msg: DaNetworkMsg<Backend, DaSharesCommitments, RuntimeServiceId>,
        backend: &mut Backend,
        membership_storage: &MembershipStorage<StorageAdapter, Membership, DaAddressbook>,
        api_adapter: &ApiAdapter,
        addressbook: &DaAddressbook,
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
            DaNetworkMsg::GetMembership {
                block_number,
                sender,
            } => {
                // todo: handle errors properly when the usage of this function is known
                // now we are just logging and returning an empty assignations
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
                    let addressbook = assignations
                        .values()
                        .flatten()
                        .filter_map(|id| {
                            addressbook
                                .get_address(id)
                                .map(|address| (*id, address))
                                .or_else(|| {
                                    tracing::error!("No address found for peer {id:?}");
                                    None
                                })
                        })
                        .collect::<AddressBookSnapshot<_>>();
                    sender.send(MembershipResponse { assignations, addressbook }).unwrap_or_else(|_| {
                                tracing::warn!(
                                    "client hung up before a subnetwork assignations handle could be established"
                                );
                            });
                } else {
                    tracing::warn!("No membership found for block number {block_number}");
                    sender.send(MembershipResponse{
                                assignations: SubnetworkAssignations::default(),
                                addressbook: AddressBookSnapshot::default(),
                            }).unwrap_or_else(|_| {
                                tracing::warn!(
                                    "client hung up before a subnetwork assignations handle could be established"
                                );
                            });
                }
            }
            DaNetworkMsg::GetCommitments { blob_id, sender } => {
                if let Err(e) = api_adapter.request_commitments(blob_id, sender).await {
                    tracing::error!("Failed to request commitments: {e}");
                }
            }
            DaNetworkMsg::RequestHistoricSample {
                block_number,
                blob_ids,
                block_id,
            } => {
                Self::handle_historic_sample_request(
                    backend,
                    membership_storage,
                    block_number,
                    block_id,
                    blob_ids,
                )
                .await;
            }
        }
    }

    async fn handle_membership_update(
        block_number: BlockNumber,
        update: HashMap<Membership::Id, Multiaddr>,
        storage: &MembershipStorage<StorageAdapter, Membership, DaAddressbook>,
    ) {
        storage
            .update(block_number, update)
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Failed to update membership at block {block_number}: {e}");
            });
    }
    async fn handle_historic_sample_request(
        backend: &Backend,
        membership_storage: &MembershipStorage<StorageAdapter, Membership, AddressBook>,
        block_number: u64,
        block_id: HeaderId,
        blob_ids: HashSet<[u8; 32]>,
    ) {
        let membership = membership_storage
            .get_historic_membership(block_number)
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Failed to get historic membership for block {block_number}: {e}");
                None
            });

        if let Some(membership) = membership {
            let send =
                backend.start_historic_sampling(block_number, block_id, blob_ids, membership);
            send.await;
        } else {
            tracing::error!("No membership found for block number {block_number}");
        }
    }
}

impl<
        Backend: NetworkBackend<RuntimeServiceId>,
        Membership,
        ApiAdapterSettings,
        RuntimeServiceId,
    > Clone for NetworkConfig<Backend, Membership, ApiAdapterSettings, RuntimeServiceId>
where
    Membership: Clone,
    ApiAdapterSettings: Clone,
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            membership: self.membership.clone(),
            api_adapter_settings: self.api_adapter_settings.clone(),
        }
    }
}

impl<
        Backend: NetworkBackend<RuntimeServiceId>,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
        RuntimeServiceId,
    > Clone
    for NetworkState<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
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
        ApiAdapter,
        RuntimeServiceId,
    > ServiceState
    for NetworkState<
        Backend,
        Membership,
        MembershipServiceAdapter,
        StorageAdapter,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    Membership: Clone,
    ApiAdapter: ApiAdapterTrait,
{
    type Settings = NetworkConfig<Backend, Membership, ApiAdapter::Settings, RuntimeServiceId>;
    type Error = <Backend::State as ServiceState>::Error;

    fn from_settings(settings: &Self::Settings) -> Result<Self, Self::Error> {
        Backend::State::from_settings(&settings.backend).map(|backend| Self {
            backend,
            phantom: PhantomData,
        })
    }
}
