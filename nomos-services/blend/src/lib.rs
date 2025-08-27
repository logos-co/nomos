use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use futures::StreamExt as _;
#[cfg(feature = "libp2p")]
use libp2p::PeerId;
use nomos_blend_scheduling::{
    membership::{Membership, Node},
    session::{SessionEvent, SessionEventStream},
};
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use serde::{Deserialize, Serialize};
use services_utils::wait_until_services_are_ready;
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;
use tracing::{debug, error, info};

use crate::settings::TimingSettings;

pub mod core;
pub mod edge;
pub mod message;
pub mod settings;

#[cfg(test)]
mod test_utils;

const LOG_TARGET: &str = "blend::service";

pub struct BlendService<CoreService, EdgeService, NodeId, RuntimeServiceId>
where
    CoreService: ServiceData,
    EdgeService: ServiceData,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(CoreService, EdgeService, NodeId)>,
}

impl<CoreService, EdgeService, NodeId, RuntimeServiceId> ServiceData
    for BlendService<CoreService, EdgeService, NodeId, RuntimeServiceId>
where
    CoreService: ServiceData,
    EdgeService: ServiceData,
{
    type Settings = Settings<NodeId>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = <CoreService as ServiceData>::Message;
}

#[async_trait]
impl<CoreService, EdgeService, NodeId, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<CoreService, EdgeService, NodeId, RuntimeServiceId>
where
    CoreService: ServiceData + Send,
    <CoreService as ServiceData>::Message: Send + 'static,
    EdgeService: ServiceData<Message = <CoreService as ServiceData>::Message> + Send,
    NodeId: Clone + Send + Sync + 'static,
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
        let settings = self
            .service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        let mut session_stream = session_stream(
            settings.time.session_duration(),
            settings.time.session_transition_period(),
            settings.membership(),
        );

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

        loop {
            tokio::select! {
                Some(message) = inbound_relay.next() => {
                    debug!(target: LOG_TARGET, "Relaying a message to core service");
                    if let Err((e, _)) = core_relay.send(message).await {
                        error!(target: LOG_TARGET, "Failed to relay message to core service: {e:?}");
                    }
                }
                Some(SessionEvent::NewSession(_membership)) = session_stream.next() => {
                    // TODO: Decide which service (core or edge) to use for the new session: https://github.com/logos-co/nomos/issues/1535
                }
            }
        }
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
        PeerId,
        RuntimeServiceId,
    >
{
    type BroadcastSettings =
        <core::network::libp2p::Libp2pAdapter<RuntimeServiceId> as core::network::NetworkAdapter<
            RuntimeServiceId,
        >>::BroadcastSettings;
}

// TODO: Replace with a stream from the membership service: https://github.com/logos-co/nomos/issues/1462?issue=logos-co%7Cnomos%7C1532
pub(crate) fn session_stream<NodeId>(
    session_duration: Duration,
    transition_period: Duration,
    membership: Membership<NodeId>,
) -> SessionEventStream<Membership<NodeId>>
where
    NodeId: Clone + Send + Sync + 'static,
{
    SessionEventStream::new(
        IntervalStream::new(interval(session_duration)).map(move |_| membership.clone()),
        transition_period,
    )
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings<NodeId> {
    pub time: TimingSettings,
    pub membership: Vec<Node<NodeId>>,
}

impl<NodeId> Settings<NodeId>
where
    NodeId: Clone,
{
    #[must_use]
    pub fn membership(&self) -> Membership<NodeId> {
        Membership::new(&self.membership, None)
    }
}
