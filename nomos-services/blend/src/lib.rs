use std::{
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use futures::StreamExt as _;
use nomos_blend_scheduling::{
    membership::Membership,
    session::{SessionEvent, SessionEventStream},
};
use nomos_network::NetworkService;
use overwatch::{
    services::{
        relay::OutboundRelay,
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use services_utils::wait_until_services_are_ready;
use tracing::{debug, error, info};

use crate::{
    core::{
        network::NetworkAdapter as NetworkAdapterTrait,
        service_components::{
            MessageComponents, NetworkBackendOfService, ServiceComponents as CoreServiceComponents,
        },
    },
    membership::Adapter as _,
    settings::{constant_membership_stream, Settings},
};

pub mod core;
pub mod edge;
pub mod message;
pub mod settings;

pub mod membership;
mod service_components;
pub use service_components::ServiceComponents;

#[cfg(test)]
mod test_utils;

const LOG_TARGET: &str = "blend::service";

pub struct BlendService<CoreService, EdgeService, RuntimeServiceId>
where
    CoreService: ServiceData + CoreServiceComponents<RuntimeServiceId>,
    EdgeService: ServiceData,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(CoreService, EdgeService)>,
}

impl<CoreService, EdgeService, RuntimeServiceId> ServiceData
    for BlendService<CoreService, EdgeService, RuntimeServiceId>
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
impl<CoreService, EdgeService, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<CoreService, EdgeService, RuntimeServiceId>
where
    CoreService: ServiceData<Message: MessageComponents<Payload: Into<Vec<u8>>> + Send + 'static>
        + CoreServiceComponents<
            RuntimeServiceId,
            NetworkAdapter: NetworkAdapterTrait<
                RuntimeServiceId,
                BroadcastSettings = BroadcastSettings<CoreService>,
            > + Send
                                + Sync,
            NodeId: Clone + Hash + Eq + Send + Sync + 'static,
        > + Send,
    EdgeService: ServiceData<Message = CoreService::Message> + edge::ServiceComponents + Send,
    EdgeService::MembershipAdapter: membership::Adapter + Send,
    <EdgeService::MembershipAdapter as membership::Adapter>::Error: Send + Sync + 'static,
    membership::ServiceMessage<EdgeService::MembershipAdapter>: Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<CoreService>
        + AsServiceId<EdgeService>
        + AsServiceId<MembershipService<EdgeService>>
        + AsServiceId<
            NetworkService<
                NetworkBackendOfService<CoreService, RuntimeServiceId>,
                RuntimeServiceId,
            >,
        > + Debug
        + Display
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

        let _membership_stream = <MembershipAdapter<EdgeService> as membership::Adapter>::new(
            overwatch_handle
                .relay::<MembershipService<EdgeService>>()
                .await?,
            settings.crypto.signing_private_key.public_key(),
        )
        .subscribe()
        .await?;
        // TODO: Use membership_stream once the membership/SDP services are ready to provide the real membership: https://github.com/logos-co/nomos/issues/1532

        let mut membership = settings.membership();
        let mut session_stream = SessionEventStream::new(
            Box::pin(constant_membership_stream(
                membership.clone(),
                settings.time.session_duration(),
            )),
            settings.time.session_transition_period(),
        );

        // Relays to the core and edge services.
        let core_relay = overwatch_handle.relay::<CoreService>().await?;
        let edge_relay = overwatch_handle.relay::<EdgeService>().await?;

        // A network adapter for broadcasting messages directly via gossipsub
        // when the Blend network is too small.
        let network_adapter = <CoreService::NetworkAdapter as NetworkAdapterTrait<
            RuntimeServiceId,
        >>::new(
            overwatch_handle
                .relay::<NetworkService<NetworkBackendOfService<CoreService, RuntimeServiceId>, _>>(
                )
                .await?,
        );

        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            CoreService,
            EdgeService,
            MembershipService<EdgeService>
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
                Some(SessionEvent::NewSession(new_membership)) = session_stream.next() => {
                    debug!(target: LOG_TARGET, "Received a new session: membership_size:{}", new_membership.size());
                    membership = new_membership;
                },
                Some(message) = inbound_relay.next() => {
                    if let Err(e) = Self::handle_inbound_message(
                        message, &membership, minimal_network_size, &core_relay, &edge_relay, &network_adapter,
                    ).await {
                        error!(target: LOG_TARGET, "Failed to handle inbound message: {e:?}");
                    }
                },
            }
        }
    }
}

type BroadcastSettings<CoreService> =
    <<CoreService as ServiceData>::Message as MessageComponents>::BroadcastSettings;

type MembershipAdapter<EdgeService> = <EdgeService as edge::ServiceComponents>::MembershipAdapter;

type MembershipService<EdgeService> =
    <MembershipAdapter<EdgeService> as membership::Adapter>::Service;

impl<CoreService, EdgeService, RuntimeServiceId>
    BlendService<CoreService, EdgeService, RuntimeServiceId>
where
    CoreService: ServiceData<Message: MessageComponents<Payload: Into<Vec<u8>>> + Send + 'static>
        + CoreServiceComponents<
            RuntimeServiceId,
            NetworkAdapter: NetworkAdapterTrait<
                RuntimeServiceId,
                BroadcastSettings = BroadcastSettings<CoreService>,
            > + Send
                                + Sync,
            NodeId: Clone + Hash + Eq + Send + Sync + 'static,
        > + Send,
    EdgeService: ServiceData<Message = CoreService::Message>,
{
    async fn handle_inbound_message(
        message: CoreService::Message,
        membership: &Membership<CoreService::NodeId>,
        minimal_network_size: usize,
        core_relay: &OutboundRelay<CoreService::Message>,
        edge_relay: &OutboundRelay<EdgeService::Message>,
        network_adapter: &CoreService::NetworkAdapter,
    ) -> Result<(), DynError> {
        if membership.size() < minimal_network_size {
            info!(target: LOG_TARGET, "Blend network too small. Broadcasting via gossipsub.");
            Self::broadcast_message(message, network_adapter).await;
            return Ok(());
        }

        if !membership.contains_local() {
            debug!(target: LOG_TARGET, "Relaying a message to edge service");
            return edge_relay.send(message).await.map_err(|(e, _)| e.into());
        }

        debug!(target: LOG_TARGET, "Relaying a message to core service");
        core_relay.send(message).await.map_err(|(e, _)| e.into())
    }

    async fn broadcast_message(
        message: CoreService::Message,
        network_adapter: &CoreService::NetworkAdapter,
    ) {
        let (payload, broadcast_settings) = message.into_components();
        network_adapter
            .broadcast(payload.into(), broadcast_settings)
            .await;
    }
}
