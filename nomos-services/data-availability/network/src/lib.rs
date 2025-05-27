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
use nomos_membership::MembershipProviders;
use nomos_sdp_core::ServiceType;
use overwatch::{
    services::{
        state::{NoOperator, ServiceState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceStateHandle,
};
use rand::seq::IteratorRandom as _;
use serde::{Deserialize, Serialize};
use services_utils::overwatch::lifecycle;
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
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
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
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            service_state,
            _phantom: PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            mut service_state, ..
        } = self;

        let network_settings = service_state.settings_reader.get_updated_settings();

        let mut backend = NetworkBackend::new(
            network_settings.backend,
            service_state.overwatch_handle.clone(),
        );

        let membership_relay = service_state
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

        let mut lifecycle_stream = service_state.lifecycle_handle.message_stream();
        loop {
            tokio::select! {
                Some(msg) = service_state.inbound_relay.recv() => {
                    Self::handle_network_service_message(msg, &mut backend).await;
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
                        // TODO: Maybe add a call to backend to handle this. Maybe trying to save unprocessed messages?
                        backend.shutdown();
                        break;
                    }
                }
                Some(update) = stream.next() => {
                    Self::handle_membership_update_message(&mut backend, &update);
                }
            }
        }

        Ok(())
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
                // need to hold an instance of &I (which is not send) across an await point
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
        if update.is_empty() {
            tracing::debug!("Received empty membership update, skipping.");
            return;
        }

        let mut members = Vec::new();
        let mut addressbook: HashMap<PeerId, Multiaddr> = HashMap::new();
        let mut rng = rand::thread_rng();

        for (provider_id, locators) in update {
            let peer_id = PeerId::from_bytes(&provider_id.0).expect("Invalid ProviderId");
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
