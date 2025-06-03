pub mod backends;

use std::{
    collections::HashMap,
    fmt::{self, Debug, Display},
    marker::PhantomData,
    pin::Pin,
};

use adapters::membership::MembershipAdapter;
use async_trait::async_trait;
use backends::NetworkBackend;
use futures::{Stream, StreamExt as _};
use libp2p::{Multiaddr, PeerId};
use nomos_libp2p::ed25519::PublicKey;
use nomos_membership::MembershipProviders;
use nomos_sdp_core::ServiceType;
use overwatch::{
    services::{
        state::{NoOperator, ServiceState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use rand::seq::IteratorRandom as _;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

pub mod adapters;

pub enum DaNetworkMsg<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> {
    Process(B::Message),
    Subscribe {
        kind: B::EventKind,
        sender: oneshot::Sender<Pin<Box<dyn Stream<Item = B::NetworkEvent> + Send>>>,
    },
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> Debug
    for DaNetworkMsg<B, RuntimeServiceId>
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Process(msg) => write!(fmt, "DaNetworkMsg::Process({msg:?})"),
            Self::Subscribe { kind, .. } => {
                write!(fmt, "DaNetworkMsg::Subscribe{{ kind: {kind:?}}}")
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct NetworkConfig<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> {
    pub backend: B::Settings,
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> Debug
    for NetworkConfig<B, RuntimeServiceId>
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "NetworkConfig {{ backend: {:?}}}", self.backend)
    }
}

pub struct NetworkService<
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Membership: MembershipAdapter + Send + 'static,
    RuntimeServiceId,
> {
    _phantom: PhantomData<(Backend, Membership)>,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
}

pub struct NetworkState<Backend: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> {
    backend: Backend::State,
}

impl<Backend, Membership, RuntimeServiceId> ServiceData
    for NetworkService<Backend, Membership, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId> + 'static + Send,
    Membership: MembershipAdapter + Send + 'static,
{
    type Settings = NetworkConfig<Backend, RuntimeServiceId>;
    type State = NetworkState<Backend, RuntimeServiceId>;
    type StateOperator = NoOperator<Self::State>;
    type Message = DaNetworkMsg<Backend, RuntimeServiceId>;
}

#[async_trait]
impl<Backend, Membership, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for NetworkService<Backend, Membership, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + Sync + 'static,
    Membership: MembershipAdapter + Send + Sync + 'static,
    <Membership::MembershipService as ServiceData>::Message: 'static,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<Membership::MembershipService>
        + Clone
        + Display
        + Send
        + Sync
        + Debug
        + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            _phantom: PhantomData,
            service_resources_handle,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            ref mut service_resources_handle,
            ..
        } = self;

        let network_settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let membership_relay = service_resources_handle
            .overwatch_handle
            .relay::<Membership::MembershipService>()
            .await?;

        let membership_adapter = Membership::new(membership_relay);

        let mut stream = membership_adapter
            .subscribe(ServiceType::DataAvailability)
            .await
            .map_err(|e| {
                tracing::error!("Failed to subscribe to membership service: {:?}", e);
                e
            })?;

        let mut backend = NetworkBackend::new(
            network_settings.backend,
            service_resources_handle.overwatch_handle.clone(),
        );

        loop {
            tokio::select! {
                Some(msg) = service_resources_handle.inbound_relay.recv() => {
                    Self::handle_network_service_message(msg, &mut backend).await;
                }
                Some(update) = stream.next() => {
                    Self::handle_membership_update_message(&mut backend, &update);
                }
            }
        }
    }
}

impl<Backend, Membership, RuntimeServiceId> NetworkService<Backend, Membership, RuntimeServiceId>
where
    Backend: NetworkBackend<RuntimeServiceId> + Send + 'static,
    Backend::State: Send + Sync,
    Membership: MembershipAdapter + Send + 'static,
    RuntimeServiceId: Send + Sync + 'static,
{
    async fn handle_network_service_message(
        msg: DaNetworkMsg<Backend, RuntimeServiceId>,
        backend: &mut Backend,
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
        }
    }

    fn handle_membership_update_message(backend: &mut Backend, update: &MembershipProviders) {
        let mut members = Vec::new();
        let mut addressbook: HashMap<PeerId, Multiaddr> = HashMap::new();
        let mut rng = rand::thread_rng();

        for (provider_id, locators) in update {
            let public_key = match PublicKey::try_from_bytes(&provider_id.0) {
                Ok(key) => key,
                Err(e) => {
                    tracing::error!("Failed to convert provider ID to public key: {e}");
                    continue;
                }
            };

            let peer_id = PeerId::from_public_key(&public_key.into());
            members.push(peer_id);

            // Pick a random locator from the BTreeSet
            if let Some(random_locator) = locators.iter().choose(&mut rng) {
                let multi_addr = random_locator.addr();
                addressbook.insert(peer_id, multi_addr.clone());
            }
        }

        backend.update_membership(members, addressbook);
    }
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> Clone
    for NetworkConfig<B, RuntimeServiceId>
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
        }
    }
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> Clone
    for NetworkState<B, RuntimeServiceId>
{
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
        }
    }
}

impl<B: NetworkBackend<RuntimeServiceId>, RuntimeServiceId> ServiceState
    for NetworkState<B, RuntimeServiceId>
{
    type Settings = NetworkConfig<B, RuntimeServiceId>;
    type Error = <B::State as ServiceState>::Error;

    fn from_settings(settings: &Self::Settings) -> Result<Self, Self::Error> {
        B::State::from_settings(&settings.backend).map(|backend| Self { backend })
    }
}
