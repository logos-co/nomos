use std::{
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use futures::{FutureExt as _, StreamExt as _, future::BoxFuture};
pub use nomos_blend_message::{
    crypto::proofs::{
        RealProofsVerifier, quota::inputs::prove::private::ProofOfLeadershipQuotaInputs,
    },
    encap::ProofsVerifier,
};
use nomos_blend_scheduling::{
    membership::Membership,
    session::{SessionEvent, UninitializedSessionEventStream},
};
use nomos_network::NetworkService;
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        state::{NoOperator, NoState},
    },
};
use services_utils::wait_until_services_are_ready;
use tracing::{debug, error, info};

use crate::{
    core::{
        mode_components::{
            BlendBackendSettingsOfMode, MessageComponents, ModeComponents as CoreModeComponents,
            NetworkBackendOfMode,
        },
        network::NetworkAdapter as NetworkAdapterTrait,
    },
    edge::mode_components::ModeComponents as EdgeModeComponents,
    membership::{Adapter as _, MembershipInfo},
    mode::Mode,
    settings::{FIRST_STREAM_ITEM_READY_TIMEOUT, Settings},
};

pub mod core;
pub mod edge;
pub mod epoch_info;
pub mod membership;
pub mod message;
pub mod session;
pub mod settings;

pub mod broadcast;
mod merkle;
mod mode;
mod service_components;
pub use self::service_components::ServiceComponents;

#[cfg(test)]
mod test_utils;

const LOG_TARGET: &str = "blend::service";

pub struct BlendService<CoreMode, EdgeMode, BroadcastMode, RuntimeServiceId>
where
    CoreMode: Mode<RuntimeServiceId> + CoreModeComponents<RuntimeServiceId>,
    EdgeMode: EdgeModeComponents,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(CoreMode, EdgeMode)>,
}

impl<CoreMode, EdgeMode, BroadcastMode, RuntimeServiceId> ServiceData
    for BlendService<CoreMode, EdgeMode, BroadcastMode, RuntimeServiceId>
where
    CoreMode: Mode<RuntimeServiceId> + CoreModeComponents<RuntimeServiceId>,
    EdgeMode: EdgeModeComponents,
{
    type Settings = Settings<
        BlendBackendSettingsOfMode<CoreMode, RuntimeServiceId>,
        <EdgeMode as EdgeModeComponents>::BackendSettings,
    >;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = CoreMode::Message;
}

#[async_trait]
impl<CoreMode, EdgeMode, BroadcastMode, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<CoreMode, EdgeMode, BroadcastMode, RuntimeServiceId>
where
    CoreMode: Mode<
            RuntimeServiceId,
            Settings: From<Self::Settings>,
            Message: MessageComponents<Payload: Into<Vec<u8>>> + Send + Sync + 'static,
        > + CoreModeComponents<
            RuntimeServiceId,
            NetworkAdapter: NetworkAdapterTrait<
                RuntimeServiceId,
                BroadcastSettings = BroadcastSettings<CoreMode, RuntimeServiceId>,
            > + Send
                                + Sync
                                + 'static,
            NodeId: Clone + Hash + Eq + Send + Sync + 'static,
            BackendSettings: Clone + Send + Sync,
        > + Send
        + 'static,
    EdgeMode: Mode<RuntimeServiceId, Settings: From<Self::Settings>, Message = CoreMode::Message>
        // We tie the core and edge proofs generator to be the same type, to avoid mistakes in the
        // node configuration where the two services use different verification logic
        + EdgeModeComponents<
            BackendSettings: Clone + Send + Sync,
            ProofsGenerator = CoreMode::ProofsGenerator,
        > + Send
        + 'static,
    EdgeMode::MembershipAdapter:
        membership::Adapter<NodeId = CoreMode::NodeId, Error: Send + Sync + 'static> + Send,
    membership::ServiceMessage<EdgeMode::MembershipAdapter>: Send + Sync + 'static,
    BroadcastMode: Mode<RuntimeServiceId, Settings: From<Self::Settings>, Message = CoreMode::Message>
        + 'static,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<MembershipService<EdgeMode>>
        + AsServiceId<
            NetworkService<NetworkBackendOfMode<CoreMode, RuntimeServiceId>, RuntimeServiceId>,
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
            service_resources_handle,
            ..
        } = self;

        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        let minimal_network_size = settings.common.minimum_network_size.get() as usize;

        wait_until_services_are_ready!(
            &service_resources_handle.overwatch_handle,
            Some(Duration::from_secs(60)),
            MembershipService<EdgeMode>
        )
        .await?;

        let membership_stream = <MembershipAdapter<EdgeMode> as membership::Adapter>::new(
            service_resources_handle
                .overwatch_handle
                .relay::<MembershipService<EdgeMode>>()
                .await?,
            settings
                .common
                .crypto
                .non_ephemeral_signing_key
                .public_key(),
            // We don't need to generate secret zk info in the proxy service, so we ignore the
            // secret key at this level.
            None,
        )
        .subscribe()
        .await?;

        let (MembershipInfo { mut membership, .. }, mut remaining_session_stream) =
            UninitializedSessionEventStream::new(
                membership_stream,
                FIRST_STREAM_ITEM_READY_TIMEOUT,
                settings.common.time.session_transition_period(),
            )
            .await_first_ready()
            .await
            .expect("The current session must be ready");

        info!(
            target: LOG_TARGET,
            "The current membership is ready: {} nodes.",
            membership.size()
        );

        let mut mode_runner = Box::pin(run_mode::<
            Self,
            _,
            CoreMode,
            EdgeMode,
            BroadcastMode,
            RuntimeServiceId,
        >(
            service_resources_handle,
            &membership,
            minimal_network_size,
        ));

        loop {
            tokio::select! {
                result = &mut mode_runner => {
                    match result {
                        Ok(service_resources_handle) => {
                            info!(target: LOG_TARGET, "Mode finished. Switching to a new mode.");
                            mode_runner = Box::pin(run_mode::<
                                Self,
                                _,
                                CoreMode,
                                EdgeMode,
                                BroadcastMode,
                                RuntimeServiceId,
                            >(
                                service_resources_handle,
                                &membership,
                                minimal_network_size,
                            ));
                        },
                        Err(e) => {
                            error!(target: LOG_TARGET, "Error while running mode: {e}");
                            return Err(e);
                        },
                    }
                },
                Some(SessionEvent::NewSession(membership_info)) = remaining_session_stream.next() => {
                    debug!(target: LOG_TARGET, "Received a new session event");
                    membership = membership_info.membership;
                },
            }
        }
    }
}

fn run_mode<Service, NodeId, CoreMode, EdgeMode, BroadcastMode, RuntimeServiceId>(
    service_resources_handle: OpaqueServiceResourcesHandle<Service, RuntimeServiceId>,
    membership: &Membership<NodeId>,
    minimal_network_size: usize,
) -> BoxFuture<'static, Result<OpaqueServiceResourcesHandle<Service, RuntimeServiceId>, DynError>>
where
    Service: ServiceData<
            Settings: Into<CoreMode::Settings>
                          + Into<EdgeMode::Settings>
                          + Into<BroadcastMode::Settings>
                          + Clone
                          + Send
                          + Sync
                          + 'static,
            State: Send + Sync + 'static,
            Message = CoreMode::Message,
        > + 'static,
    CoreMode: Mode<RuntimeServiceId, Message: Send + 'static> + 'static,
    EdgeMode: Mode<RuntimeServiceId, Message = Service::Message> + 'static,
    BroadcastMode: Mode<RuntimeServiceId, Message = Service::Message> + 'static,
    RuntimeServiceId: 'static,
{
    if membership.size() < minimal_network_size {
        info!(target: LOG_TARGET, "Running in Broadcast mode...");
        BroadcastMode::run::<Service>(service_resources_handle).boxed()
    } else if membership.contains_local() {
        info!(target: LOG_TARGET, "Running in Core mode...");
        CoreMode::run::<Service>(service_resources_handle).boxed()
    } else {
        info!(target: LOG_TARGET, "Running in Edge mode...");
        EdgeMode::run::<Service>(service_resources_handle).boxed()
    }
}

type BroadcastSettings<CoreMode, RuntimeServiceId> =
    <<CoreMode as Mode<RuntimeServiceId>>::Message as MessageComponents>::BroadcastSettings;

type MembershipAdapter<EdgeMode> = <EdgeMode as edge::ModeComponents>::MembershipAdapter;

type MembershipService<EdgeMode> = <MembershipAdapter<EdgeMode> as membership::Adapter>::Service;
