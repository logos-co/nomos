use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use futures::StreamExt as _;
#[cfg(feature = "libp2p")]
use libp2p::PeerId;
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
        service_components::{MessageComponents, NetworkBackendOfService, ServiceComponents},
    },
    settings::NetworkSettings,
};

pub mod core;
pub mod edge;
pub mod message;
pub mod settings;

#[cfg(test)]
mod test_utils;

const LOG_TARGET: &str = "blend::service";

pub struct BlendService<CoreService, EdgeService, RuntimeServiceId>
where
    CoreService: ServiceData + ServiceComponents<RuntimeServiceId>,
    EdgeService: ServiceData,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(CoreService, EdgeService)>,
}

impl<CoreService, EdgeService, RuntimeServiceId> ServiceData
    for BlendService<CoreService, EdgeService, RuntimeServiceId>
where
    CoreService: ServiceData + ServiceComponents<RuntimeServiceId>,
    EdgeService: ServiceData,
{
    type Settings = NetworkSettings<CoreService::NodeId>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = CoreService::Message;
}

#[async_trait]
impl<CoreService, EdgeService, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<CoreService, EdgeService, RuntimeServiceId>
where
    CoreService: ServiceData<Message: MessageComponents<Payload: Into<Vec<u8>>> + Send + 'static>
        + ServiceComponents<
            RuntimeServiceId,
            NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId, BroadcastSettings = <<CoreService as ServiceData>::Message as MessageComponents>::BroadcastSettings> + Send,
            NodeId: Clone + Send + Sync,
        > + Send,
    EdgeService: ServiceData<Message = <CoreService as ServiceData>::Message> + Send,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<CoreService>
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
        let settings = self
            .service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        // TODO: Add logic to start/stop the core or edge service based on the new
        // membership info.

        let minimal_network_size = settings.minimal_network_size;
        let membership_size = settings.membership.len();

        let inbound_relay = &mut self.service_resources_handle.inbound_relay;

        if membership_size >= minimal_network_size.get() as usize {
            wait_until_services_are_ready!(
                &self.service_resources_handle.overwatch_handle,
                Some(Duration::from_secs(60)),
                CoreService
            )
            .await?;
            let core_relay = self
                .service_resources_handle
                .overwatch_handle
                .relay::<CoreService>()
                .await?;
            self.service_resources_handle.status_updater.notify_ready();
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
                &self.service_resources_handle.overwatch_handle,
                Some(Duration::from_secs(60)),
                NetworkService<NetworkBackendOfService<CoreService, RuntimeServiceId>, _>
            )
            .await?;
            let core_network_relay = self
                .service_resources_handle
                .overwatch_handle
                .relay::<NetworkService<NetworkBackendOfService<CoreService, RuntimeServiceId>, _>>(
                )
                .await?;
            // A network adapter that uses the same settings as the core Blend service.
            let core_network_adapter = <CoreService::NetworkAdapter as NetworkAdapterTrait<
                RuntimeServiceId,
            >>::new(core_network_relay);
            self.service_resources_handle.status_updater.notify_ready();
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

/// Defines additional types required for communicating with [`BlendService`].
///
/// In particular, this trait extends the [`ServiceData`] by introducing types
/// that are not covered by [`ServiceData`] but are necessary to construct
/// messages send to a [`BlendService`].
pub trait ServiceExt {
    /// A type for broadcast settings required for
    /// [`crate::message::ServiceMessage`].
    ///
    /// This depends on the the [`core::network::NetworkAdapter`] of the
    /// [`core::BlendService`] that is not exposed to the users of
    /// [`BlendService`].
    /// Therefore, this type must be specified for [`BlendService`]s
    /// that depend on concrete types of [`core::network::NetworkAdapter`].
    type BroadcastSettings;
}

/// Implementing [`ServiceExt`] for [`BlendService`]
/// that depends on the libp2p-based [`core::BlendService`] and
/// [`edge::BlendService`].
#[cfg(feature = "libp2p")]
impl<RuntimeServiceId> ServiceExt
    for BlendService<
        core::BlendService<
            core::backends::libp2p::Libp2pBlendBackend,
            PeerId,
            core::network::libp2p::Libp2pAdapter<RuntimeServiceId>,
            RuntimeServiceId,
        >,
        edge::BlendService<edge::backends::libp2p::Libp2pBlendBackend, PeerId,
            <core::network::libp2p::Libp2pAdapter<RuntimeServiceId> as core::network::NetworkAdapter<RuntimeServiceId>>::BroadcastSettings, RuntimeServiceId>,
        RuntimeServiceId,
    >
{
    type BroadcastSettings =
        <core::network::libp2p::Libp2pAdapter<RuntimeServiceId> as core::network::NetworkAdapter<
            RuntimeServiceId,
        >>::BroadcastSettings;
}
