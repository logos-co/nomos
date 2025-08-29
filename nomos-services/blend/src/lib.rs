use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use futures::StreamExt as _;
use nomos_network::NetworkService;
use overwatch::{
    services::{
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
    settings::Settings,
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
            NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId, BroadcastSettings = <<CoreService as ServiceData>::Message as MessageComponents>::BroadcastSettings> + Send,
            NodeId: Clone + Send + Sync,
        > + Send,
    EdgeService: ServiceData<Message = <CoreService as ServiceData>::Message> + edge::ServiceComponents +  Send,
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

        let membership_adapter = <MembershipAdapter<EdgeService> as membership::Adapter>::new(
            overwatch_handle
                .relay::<MembershipService<EdgeService>>()
                .await?,
            settings.crypto.signing_private_key.public_key(),
        );
        let mut _membership_stream = membership_adapter.subscribe().await?;
        // TODO: Use membership_stream as a session stream: https://github.com/logos-co/nomos/issues/1532

        // TODO: Add logic to start/stop the core or edge service based on the new
        // membership info.

        let minimal_network_size = settings.minimal_network_size;
        let membership_size = settings.membership.len();

        if membership_size >= minimal_network_size.get() as usize {
            wait_until_services_are_ready!(
                &overwatch_handle,
                Some(Duration::from_secs(60)),
                CoreService,
                MembershipService<EdgeService>
            )
            .await?;
            let core_relay =
                overwatch_handle
                .relay::<CoreService>()
                .await?;
            status_updater.notify_ready();
            info!(
                target: LOG_TARGET,
                "Service '{}' is ready.",
                <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
            );
            while let Some(message) = inbound_relay.next().await {
                debug!(target: LOG_TARGET, "Relaying a message to core service");
                if let Err((e, _)) = core_relay.send(message).await {
                    error!(target: LOG_TARGET, "Failed to relay message to core service: {e:?}");
                }
            }
        } else {
            wait_until_services_are_ready!(
                &overwatch_handle,
                Some(Duration::from_secs(60)),
                NetworkService<NetworkBackendOfService<CoreService, RuntimeServiceId>, _>,
                MembershipService<EdgeService>
            )
            .await?;
            let core_network_relay =
                overwatch_handle
                .relay::<NetworkService<NetworkBackendOfService<CoreService, RuntimeServiceId>, _>>(
                )
                .await?;
            // A network adapter that uses the same settings as the core Blend service.
            let core_network_adapter = <CoreService::NetworkAdapter as NetworkAdapterTrait<
                RuntimeServiceId,
            >>::new(core_network_relay);
            status_updater.notify_ready();
            info!(
                target: LOG_TARGET,
                "Service '{}' is ready.",
                <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
            );
            while let Some(message) = inbound_relay.next().await {
                info!(target: LOG_TARGET, "Blend network too small. Broadcasting via gossipsub.");
                let (payload, broadcast_settings) = message.into_components();
                core_network_adapter
                    .broadcast(
                        payload.into(),
                        broadcast_settings,
                    )
                    .await;
            }
        }

        Ok(())
    }
}

type MembershipAdapter<EdgeService> = <EdgeService as edge::ServiceComponents>::MembershipAdapter;

type MembershipService<EdgeService> =
    <MembershipAdapter<EdgeService> as membership::Adapter>::Service;
