use std::{
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
};

use async_trait::async_trait;
use futures::StreamExt as _;
use nomos_blend_scheduling::session::SessionEventStream;
use nomos_network::NetworkService;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use tracing::{debug, error, info};

use crate::{
    core::{
        network::NetworkAdapter as NetworkAdapterTrait,
        service_components::{
            MessageComponents, NetworkBackendOfService, ServiceComponents as CoreServiceComponents,
        },
    },
    instance::{Instance, Mode},
    settings::{constant_membership_stream, Settings},
};

pub mod core;
pub mod edge;
pub mod message;
pub mod settings;

mod instance;
pub mod membership;
mod modes;
mod service_components;
pub use service_components::ServiceComponents;

#[cfg(test)]
mod test_utils;

const LOG_TARGET: &str = "blend::service";

pub struct BlendService<CoreService, EdgeService, MembershipAdapter, RuntimeServiceId>
where
    CoreService: ServiceData + CoreServiceComponents<RuntimeServiceId>,
    EdgeService: ServiceData,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(CoreService, EdgeService)>,
}

impl<CoreService, EdgeService, MembershipAdapter, RuntimeServiceId> ServiceData
    for BlendService<CoreService, EdgeService, MembershipAdapter, RuntimeServiceId>
where
    CoreService: ServiceData + CoreServiceComponents<RuntimeServiceId>,
    EdgeService: ServiceData,
{
    type Settings = Settings<CoreService::NodeId>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = CoreService::Message;
}

#[async_trait]
impl<CoreService, EdgeService, MembershipAdapter, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<CoreService, EdgeService, MembershipAdapter, RuntimeServiceId>
where
    CoreService: ServiceData<Message: MessageComponents<Payload: Into<Vec<u8>>> + Send + Sync + 'static>
        + CoreServiceComponents<
            RuntimeServiceId,
            NetworkAdapter: NetworkAdapterTrait<
                RuntimeServiceId,
                BroadcastSettings = BroadcastSettings<CoreService>,
            > + Send
                                + Sync
                                + 'static,
            NodeId: Clone + Hash + Eq + Send + Sync + 'static,
        > + Send
        + 'static,
    EdgeService:
        ServiceData<Message = CoreService::Message> + edge::ServiceComponents + Send + 'static,
    MembershipAdapter: membership::Adapter + Send,
    <MembershipAdapter as membership::Adapter>::Error: Send + Sync + 'static,
    membership::ServiceMessage<MembershipAdapter>: Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<CoreService>
        + AsServiceId<EdgeService>
        + AsServiceId<MembershipService<MembershipAdapter>>
        + AsServiceId<
            NetworkService<
                NetworkBackendOfService<CoreService, RuntimeServiceId>,
                RuntimeServiceId,
            >,
        > + Debug
        + Display
        + Clone
        + Send
        + Sync
        + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self {
            service_resources_handle,
            _phantom: PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), DynError> {
        let Self {
            service_resources_handle:
                OpaqueServiceResourcesHandle::<Self, RuntimeServiceId> {
                    ref mut inbound_relay,
                    ref overwatch_handle,
                    ref settings_handle,
                    ref status_updater,
                    ..
                },
            ..
        } = self;

        let settings = settings_handle.notifier().get_updated_settings();
        let minimal_network_size = settings.minimal_network_size.get() as usize;

        let _membership_stream = MembershipAdapter::new(
            overwatch_handle
                .relay::<MembershipService<MembershipAdapter>>()
                .await?,
            settings.crypto.signing_private_key.public_key(),
        )
        .subscribe()
        .await?;
        // TODO: Use membership_stream once the membership/SDP services are ready to provide the real membership: https://github.com/logos-co/nomos/issues/1532

        let membership = settings.membership();
        let mut session_stream = SessionEventStream::new(
            Box::pin(constant_membership_stream(
                membership.clone(),
                settings.time.session_duration(),
            )),
            settings.time.session_transition_period(),
        );

        let mut instance = Instance::<CoreService, EdgeService, RuntimeServiceId>::new(
            Mode::choose(&membership, minimal_network_size),
            overwatch_handle,
        )
        .await?;

        status_updater.notify_ready();
        info!(
            target: LOG_TARGET,
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        loop {
            tokio::select! {
                Some(event) = session_stream.next() => {
                    debug!(target: LOG_TARGET, "Received a new session event");
                    instance = instance.handle_session_event(event, overwatch_handle, minimal_network_size).await?;
                },
                Some(message) = inbound_relay.next() => {
                    if let Err(e) = instance.handle_inbound_message(message).await {
                        error!(target: LOG_TARGET, "Failed to handle inbound message: {e:?}");
                    }
                },
            }
        }
    }
}

type BroadcastSettings<CoreService> =
    <<CoreService as ServiceData>::Message as MessageComponents>::BroadcastSettings;

type MembershipService<MembershipAdapter> = <MembershipAdapter as membership::Adapter>::Service;
