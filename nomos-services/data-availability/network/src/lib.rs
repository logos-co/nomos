pub mod addressbook;
pub mod api;
pub mod backends;
pub mod membership;
mod opinion_aggregator;
pub mod sdp;
pub mod storage;

use std::{
    collections::{HashMap, HashSet},
    fmt::{self, Debug, Display},
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use backends::{ConnectionStatus, NetworkBackend};
use futures::{Stream, stream::select};
use kzgrs_backend::common::share::{DaShare, DaSharesCommitments};
use libp2p::{Multiaddr, PeerId};
use nomos_core::{block::SessionNumber, da::BlobId, header::HeaderId, sdp::ProviderId};
use nomos_da_network_core::{
    SubnetworkId, addressbook::AddressBookHandler as _,
    protocols::sampling::opinions::OpinionEvent, swarm::BalancerStats,
};
use nomos_libp2p::cryptarchia_sync::DynError;
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        state::{NoOperator, ServiceState},
    },
};
use serde::{Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use storage::{MembershipStorage, MembershipStorageAdapter};
use subnetworks_assignations::{MembershipCreator, MembershipHandler, SubnetworkAssignations};
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};
use tokio_stream::{
    StreamExt as _,
    wrappers::{IntervalStream, ReceiverStream, UnboundedReceiverStream},
};

use crate::{
    addressbook::{AddressBook, AddressBookSnapshot},
    api::ApiAdapter as ApiAdapterTrait,
    membership::{
        MembershipAdapter, SubnetworkPeers,
        handler::{DaMembershipHandler, SharedMembershipHandler},
    },
    opinion_aggregator::{OpinionAggregator, OpinionError},
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
        session_id: SessionNumber,
        sender: oneshot::Sender<MembershipResponse>,
    },
    GetCommitments {
        blob_id: BlobId,
        sender: oneshot::Sender<Option<Commitments>>,
    },
    RequestHistoricSampling {
        session_id: SessionNumber,
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
            Self::GetMembership { session_id, .. } => {
                write!(
                    fmt,
                    "DaNetworkMsg::GetMembership{{ session_id: {session_id} }}"
                )
            }
            Self::GetCommitments { blob_id, .. } => {
                write!(
                    fmt,
                    "DaNetworkMsg::GetCommitments{{ blob_id: {blob_id:?} }}"
                )
            }
            Self::RequestHistoricSampling {
                session_id,
                blob_ids,
                block_id,
            } => {
                write!(
                    fmt,
                    "DaNetworkMsg::RequestHistoricSample{{ session_id: {session_id}, blob_ids: {blob_ids:?}, block_id: {block_id} }}"
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
    pub subnet_refresh_interval: Duration,
    // Number of connected subnetworks that the node requires to have in order to operate
    // correctly.
    pub subnet_threshold: usize,
}

impl<Backend: NetworkBackend<RuntimeServiceId>, Membership, ApiAdapterSettings, RuntimeServiceId>
    Debug for NetworkConfig<Backend, Membership, ApiAdapterSettings, RuntimeServiceId>
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
    subnet_refresh_sender: Sender<()>,
    balancer_stats_stream: UnboundedReceiverStream<BalancerStats>,
    opinion_stream: UnboundedReceiverStream<OpinionEvent>,
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

impl<Backend, Membership, MembershipServiceAdapter, StorageAdapter, ApiAdapter, RuntimeServiceId>
    ServiceData
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
impl<Backend, Membership, MembershipServiceAdapter, StorageAdapter, ApiAdapter, RuntimeServiceId>
    ServiceCore<RuntimeServiceId>
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
            HistoricMembership = SharedMembershipHandler<Membership>,
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
        + AsServiceId<StorageAdapter::StorageService>
        + 'static,
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

        // Sampling subnetwork peers need to be updatedd periodically.
        // They also need to be updated when the assignations change.
        let (subnet_refresh_sender, refresh_rx) = mpsc::channel(1);
        let interval = tokio::time::interval(settings.subnet_refresh_interval);
        let refresh_ticker = IntervalStream::new(interval).map(|_| ());
        let refresh_signal = ReceiverStream::new(refresh_rx);
        let subnet_refresh_signal = select(refresh_ticker, refresh_signal);
        let (balancer_stats_sender, balancer_stats_receiver) = mpsc::unbounded_channel();
        let balancer_stats_stream = UnboundedReceiverStream::new(balancer_stats_receiver);
        let (opinion_sender, opinion_receiver) = mpsc::unbounded_channel();
        let opinion_stream = UnboundedReceiverStream::new(opinion_receiver);

        Ok(Self {
            backend: <Backend as NetworkBackend<RuntimeServiceId>>::new(
                settings.backend,
                service_resources_handle.overwatch_handle.clone(),
                membership.clone(),
                addressbook.clone(),
                subnet_refresh_signal,
                balancer_stats_sender,
                opinion_sender,
            ),
            service_resources_handle,
            membership,
            addressbook,
            api_adapter,
            phantom: PhantomData,
            subnet_refresh_sender,
            balancer_stats_stream,
            opinion_stream,
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
            ref subnet_refresh_sender,
            ref mut balancer_stats_stream,
            ref mut opinion_stream,
            ..
        } = self;

        let membership_service_relay = overwatch_handle
            .relay::<MembershipServiceAdapter::MembershipService>()
            .await?;

        let storage_service_relay = overwatch_handle
            .relay::<StorageAdapter::StorageService>()
            .await?;

        let storage_adapter = Arc::new(StorageAdapter::new(storage_service_relay));
        let membership_storage = MembershipStorage::new(
            Arc::clone(&storage_adapter),
            membership.clone(),
            addressbook.clone(),
        );

        let membership_service_adapter = MembershipServiceAdapter::new(membership_service_relay);

        let (local_peer_id, local_provider_id) = backend.local_peer_id();
        let mut opinion_aggregator =
            OpinionAggregator::new(storage_adapter, local_peer_id, local_provider_id);

        wait_until_services_are_ready!(
            &self.service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            <MembershipServiceAdapter as MembershipAdapter>::MembershipService
        )
        .await?;

        let mut membership_updates_stream =
            membership_service_adapter.subscribe().await.map_err(|e| {
                tracing::error!("Failed to subscribe to membership service: {e}");
                e
            })?;

        let subnet_threshold = self
            .service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings()
            .subnet_threshold;

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
                Some(update) = membership_updates_stream.next() => {
                    if update.peers.is_empty() {
                        // todo: this is a short term fix, handle empty providers in assignations
                        tracing::error!("Received empty membership for session {}: skipping session update", update.session_id);
                        continue
                    }
                    tracing::debug!(
                        "Received membership update for session {}: {:?}",
                        update.session_id, update.peers
                    );

                    Self::handle_opinion_session_change(&mut opinion_aggregator, &update).await;
                    match Self::handle_membership_update(
                        update.session_id,
                        update.peers,
                        &membership_storage,
                        update.provider_mappings,
                    ).await {
                        Ok(_) => {
                            let _ = subnet_refresh_sender.send(()).await;
                        }
                        Err(e) => {
                            tracing::error!("Error handling membership update for session {}: {e}", update.session_id);
                        }
                    }
                }
                Some(stats) = balancer_stats_stream.next() => {
                    let connected_subnetworks = stats.values()
                        .filter(|stats| stats.inbound > 0 || stats.outbound > 0)
                        .count();
                    if connected_subnetworks < subnet_threshold {
                        backend.update_status(ConnectionStatus::InsufficientSubnetworkConnections);
                    } else {
                        backend.update_status(ConnectionStatus::Ready);
                    }
                }
                Some(opinion_event) = opinion_stream.next() => {
                    opinion_aggregator.record_opinion(opinion_event);
                }
            }
        }
    }
}

impl<Backend, Membership, MembershipServiceAdapter, StorageAdapter, ApiAdapter, RuntimeServiceId>
    Drop
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

impl<Backend, Membership, MembershipServiceAdapter, StorageAdapter, ApiAdapter, RuntimeServiceId>
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
    Backend: NetworkBackend<RuntimeServiceId, HistoricMembership = SharedMembershipHandler<Membership>>
        + Send
        + Sync
        + 'static,
    Backend::State: Send + Sync,
    Membership:
        MembershipCreator<Id = PeerId, NetworkId = SubnetworkId> + Clone + Send + Sync + 'static,
{
    async fn handle_network_service_message(
        msg: DaNetworkMsg<Backend, DaSharesCommitments, RuntimeServiceId>,
        backend: &mut Backend,
        membership_storage: &MembershipStorage<Arc<StorageAdapter>, Membership, DaAddressbook>,
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
            DaNetworkMsg::GetMembership { session_id, sender } => {
                // todo: handle errors properly when the usage of this function is known
                // now we are just logging and returning an empty assignations
                if let Some(membership) = membership_storage
                    .get_historic_membership(session_id)
                    .await
                    .unwrap_or_else(|e| {
                        tracing::error!(
                            "Failed to get historic membership for session {session_id}: {e}"
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
                    tracing::warn!("No membership found for session {session_id}");
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
            DaNetworkMsg::RequestHistoricSampling {
                session_id,
                blob_ids,
                block_id,
            } => {
                Self::handle_historic_sample_request(
                    backend,
                    membership_storage,
                    session_id,
                    block_id,
                    blob_ids,
                )
                .await;
            }
        }
    }

    async fn handle_membership_update(
        session_id: SessionNumber,
        update: HashMap<Membership::Id, Multiaddr>,
        storage: &MembershipStorage<Arc<StorageAdapter>, Membership, DaAddressbook>,
        provider_mappings: HashMap<Membership::Id, ProviderId>,
    ) -> Result<Membership, DynError> {
        let current_membership = storage
            .update(session_id, update, provider_mappings)
            .await?;
        Ok(current_membership)
    }

    async fn handle_historic_sample_request(
        backend: &Backend,
        membership_storage: &MembershipStorage<Arc<StorageAdapter>, Membership, AddressBook>,
        session_id: SessionNumber,
        block_id: HeaderId,
        blob_ids: HashSet<[u8; 32]>,
    ) {
        let membership = membership_storage
            .get_historic_membership(session_id)
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Failed to get historic membership for session {session_id}: {e}");
                None
            });

        if let Some(membership) = membership {
            let membership = SharedMembershipHandler::new(membership);
            let send = backend.start_historic_sampling(session_id, block_id, blob_ids, membership);
            send.await;
        } else {
            tracing::error!("No membership found for session {session_id}");
        }
    }

    async fn handle_opinion_session_change(
        opinion_aggregator: &mut OpinionAggregator<Arc<StorageAdapter>>,
        new_providers: &SubnetworkPeers<Membership::Id>,
    ) {
        match opinion_aggregator
            .handle_session_change(new_providers)
            .await
        {
            Ok(metadata) => {
                tracing::debug!(
                    "Generated opinions - session_id: {}, new_opinions: {}, old_opinions: {}, bytes {:?}",
                    metadata.current_session,
                    metadata.previous_session_opinions.len(),
                    metadata.current_session_opinions.len(),
                    metadata.to_metadata_bytes(),
                );

                //todo: sdp_adapter.post_activity(
                //metadata.to_metadata_bytes()).await;
            }
            Err(OpinionError::InsufficientData) => {
                tracing::debug!(
                    "Insufficient data to generate opinions (first session), skip forming session"
                );
            }
            Err(OpinionError::Error(e)) => {
                tracing::error!("Failed to generate opinions: {e}");
            }
        }
    }
}

impl<Backend: NetworkBackend<RuntimeServiceId>, Membership, ApiAdapterSettings, RuntimeServiceId>
    Clone for NetworkConfig<Backend, Membership, ApiAdapterSettings, RuntimeServiceId>
where
    Membership: Clone,
    ApiAdapterSettings: Clone,
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            membership: self.membership.clone(),
            api_adapter_settings: self.api_adapter_settings.clone(),
            subnet_refresh_interval: self.subnet_refresh_interval,
            subnet_threshold: self.subnet_threshold,
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
