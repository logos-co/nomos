use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use futures::StreamExt as _;
use libp2p::PeerId;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use services_utils::wait_until_services_are_ready;
use tracing::{debug, error, info};

pub mod core;
pub mod edge;
pub mod message;
pub mod settings;

const LOG_TARGET: &str = "blend::service";

pub struct BlendService<CoreService, EdgeService, RuntimeServiceId>
where
    CoreService: ServiceData,
    EdgeService: ServiceData,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(CoreService, EdgeService)>,
}

impl<CoreService, EdgeService, RuntimeServiceId> ServiceData
    for BlendService<CoreService, EdgeService, RuntimeServiceId>
where
    CoreService: ServiceData,
    EdgeService: ServiceData,
{
    type Settings = ();
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = <CoreService as ServiceData>::Message;
}

#[async_trait]
impl<CoreService, EdgeService, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<CoreService, EdgeService, RuntimeServiceId>
where
    CoreService: ServiceData + Send,
    <CoreService as ServiceData>::Message: Send + 'static,
    EdgeService: ServiceData<Message = <CoreService as ServiceData>::Message> + Send,
    RuntimeServiceId:
        AsServiceId<Self> + AsServiceId<CoreService> + Debug + Display + Send + Sync + 'static,
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
        wait_until_services_are_ready!(
            &self.service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            CoreService
        )
        .await?;

        let inbound_relay = &mut self.service_resources_handle.inbound_relay;
        while let Some(message) = inbound_relay.next().await {
            debug!(target: LOG_TARGET, "Relaying a message to core service");
            if let Err((e, _)) = core_relay.send(message).await {
                error!(target: LOG_TARGET, "Failed to relay message to core service: {e:?}");
            }
        }

        Ok(())
    }
}

/// Defines additional types required for communicating with [`BlendService`].
pub trait ServiceExt {
    type BroadcastSettings;
}

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
