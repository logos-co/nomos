use std::{
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use fork_stream::StreamExt as _;
use futures::StreamExt as _;
use nomos_blend_message::encap;
pub use nomos_blend_message::{
    crypto::proofs::{
        RealProofsVerifier, quota::inputs::prove::private::ProofOfLeadershipQuotaInputs,
    },
    encap::ProofsVerifier,
};
use nomos_blend_scheduling::{
    message_blend::provers::{
        core_and_leader::CoreAndLeaderProofsGenerator, leader::LeaderProofsGenerator,
    },
    session::UninitializedSessionEventStream,
    stream::UninitializedFirstReadyStream,
};
use nomos_network::NetworkService;
use nomos_time::TimeServiceMessage;
use nomos_utils::blake_rng::BlakeRng;
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        state::{NoOperator, NoState},
    },
};
use serde::Serialize;
use services_utils::wait_until_services_are_ready;
use tokio::sync::oneshot;
use tracing::info;

use crate::{
    context::Context,
    core::{CoreMode, network::NetworkAdapter as NetworkAdapterTrait},
    edge::EdgeMode,
    epoch_info::{EpochEvent, EpochHandler},
    message::ServiceMessage,
    mode::{Mode, ModeKind},
    settings::{FIRST_STREAM_ITEM_READY_TIMEOUT, Settings},
};

pub mod core;
pub mod edge;
pub mod epoch_info;
pub mod membership;
pub mod message;
pub mod session;
pub mod settings;

mod context;
mod merkle;
mod mode;
mod service_components;
pub use service_components::ServiceComponents;
#[cfg(test)]
mod test_utils;

const LOG_TARGET: &str = "blend::service";

#[expect(clippy::type_complexity, reason = "Many generics for PhatomData")]
pub struct BlendService<
    CoreBackend,
    EdgeBackend,
    BroadcastSettings,
    MembershipAdapter,
    ChainService,
    NetworkAdapter,
    TimeService,
    PolInfoProvider,
    CoreProofsGenerator,
    EdgeProofsGenerator,
    ProofsVerifier,
    RuntimeServiceId,
> where
    CoreBackend: core::backends::BlendBackend<
            <MembershipAdapter as membership::Adapter>::NodeId,
            BlakeRng,
            ProofsVerifier,
            RuntimeServiceId,
        >,
    EdgeBackend: edge::backends::BlendBackend<
            <MembershipAdapter as membership::Adapter>::NodeId,
            RuntimeServiceId,
        >,
    MembershipAdapter: membership::Adapter<NodeId: Clone>,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<(
        CoreBackend,
        EdgeBackend,
        BroadcastSettings,
        MembershipAdapter,
        ChainService,
        NetworkAdapter,
        TimeService,
        PolInfoProvider,
        CoreProofsGenerator,
        EdgeProofsGenerator,
        ProofsVerifier,
    )>,
}

impl<
    CoreBackend,
    EdgeBackend,
    BroadcastSettings,
    MembershipAdapter,
    ChainService,
    NetworkAdapter,
    TimeService,
    PolInfoProvider,
    CoreProofsGenerator,
    EdgeProofsGenerator,
    ProofsVerifier,
    RuntimeServiceId,
> ServiceData
    for BlendService<
        CoreBackend,
        EdgeBackend,
        BroadcastSettings,
        MembershipAdapter,
        ChainService,
        NetworkAdapter,
        TimeService,
        PolInfoProvider,
        CoreProofsGenerator,
        EdgeProofsGenerator,
        ProofsVerifier,
        RuntimeServiceId,
    >
where
    CoreBackend: core::backends::BlendBackend<
            <MembershipAdapter as membership::Adapter>::NodeId,
            BlakeRng,
            ProofsVerifier,
            RuntimeServiceId,
        >,
    EdgeBackend: edge::backends::BlendBackend<
            <MembershipAdapter as membership::Adapter>::NodeId,
            RuntimeServiceId,
        >,
    MembershipAdapter: membership::Adapter<NodeId: Clone>,
{
    type Settings = Settings<CoreBackend::Settings, EdgeBackend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ServiceMessage<BroadcastSettings>;
}

#[async_trait]
impl<
    CoreBackend,
    EdgeBackend,
    BroadcastSettings,
    MembershipAdapter,
    ChainService,
    NetworkAdapter,
    TimeService,
    PolInfoProvider,
    CoreProofsGenerator,
    EdgeProofsGenerator,
    ProofsVerifier,
    RuntimeServiceId,
> ServiceCore<RuntimeServiceId>
    for BlendService<
        CoreBackend,
        EdgeBackend,
        BroadcastSettings,
        MembershipAdapter,
        ChainService,
        NetworkAdapter,
        TimeService,
        PolInfoProvider,
        CoreProofsGenerator,
        EdgeProofsGenerator,
        ProofsVerifier,
        RuntimeServiceId,
    >
where
    CoreBackend: core::backends::BlendBackend<
            <MembershipAdapter as membership::Adapter>::NodeId,
            BlakeRng,
            ProofsVerifier,
            RuntimeServiceId,
        > + Send
        + Sync
        + 'static,
    EdgeBackend: edge::backends::BlendBackend<
            <MembershipAdapter as membership::Adapter>::NodeId,
            RuntimeServiceId,
        > + Send
        + Sync,
    BroadcastSettings: Serialize + Send + Unpin + 'static,
    MembershipAdapter:
        membership::Adapter<NodeId: Clone + Eq + Hash + Send + Sync + 'static> + Send,
    <<MembershipAdapter as membership::Adapter>::Service as ServiceData>::Message: Send + 'static,
    ChainService: CryptarchiaServiceData<Tx: Send + Sync> + Sync,
    NetworkAdapter: NetworkAdapterTrait<RuntimeServiceId, BroadcastSettings = BroadcastSettings>
        + Clone
        + Send
        + Sync
        + 'static,
    TimeService: ServiceData<Message = TimeServiceMessage> + Send,
    PolInfoProvider:
        epoch_info::PolInfoProvider<RuntimeServiceId, Stream: Send + Unpin + 'static> + Send,
    CoreProofsGenerator: CoreAndLeaderProofsGenerator + Send + 'static,
    EdgeProofsGenerator: LeaderProofsGenerator + Send,
    ProofsVerifier: encap::ProofsVerifier + Send + 'static,
    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<<MembershipAdapter as membership::Adapter>::Service>
        + AsServiceId<ChainService>
        + AsServiceId<
            NetworkService<
                <NetworkAdapter as NetworkAdapterTrait<RuntimeServiceId>>::Backend,
                RuntimeServiceId,
            >,
        > + AsServiceId<TimeService>
        + Debug
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

    #[expect(
        clippy::too_many_lines,
        reason = "TODO: factor out initialization steps"
    )]
    async fn run(mut self) -> Result<(), DynError> {
        let Self {
            service_resources_handle:
                OpaqueServiceResourcesHandle::<Self, RuntimeServiceId> {
                    inbound_relay,
                    ref overwatch_handle,
                    ref settings_handle,
                    ref status_updater,
                    ..
                },
            ..
        } = self;

        let settings = settings_handle.notifier().get_updated_settings();
        let minimal_network_size = settings.common.minimum_network_size.get() as usize;

        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            <MembershipAdapter as membership::Adapter>::Service,
            ChainService,
            NetworkService<
                <NetworkAdapter as NetworkAdapterTrait<RuntimeServiceId>>::Backend,
                RuntimeServiceId,
            >
        )
        .await?;

        let network_adapter = NetworkAdapter::new(
            overwatch_handle
                .relay::<NetworkService<
                    <NetworkAdapter as NetworkAdapterTrait<RuntimeServiceId>>::Backend,
                    RuntimeServiceId,
                >>()
                .await
                .expect("Relay with network service should be available."),
        );

        let mut epoch_handler = async {
            let chain_service = CryptarchiaServiceApi::<ChainService, _>::new(overwatch_handle)
                .await
                .expect("Failed to establish channel with chain service.");
            EpochHandler::new(
                chain_service,
                settings.common.time.epoch_transition_period_in_slots,
            )
        }
        .await;

        // Initialize clock stream for epoch-related public PoQ inputs.
        let clock_stream = async {
            let time_relay = overwatch_handle
                .relay::<TimeService>()
                .await
                .expect("Relay with time service should be available.");
            let (sender, receiver) = oneshot::channel();
            time_relay
                .send(TimeServiceMessage::Subscribe { sender })
                .await
                .expect("Failed to subscribe to slot clock.");
            receiver
                .await
                .expect("Should not fail to receive slot stream from time service.")
        }
        .await;
        let (current_leader_inputs_minus_quota, current_clock, clock_stream) = async {
            let (clock_tick, clock_stream) =
                UninitializedFirstReadyStream::new(clock_stream, Duration::from_secs(5))
                    .first()
                    .await
                    .expect("The clock system must be available.");
            let Some(EpochEvent::NewEpoch(new_epoch_info)) = epoch_handler.tick(clock_tick).await
            else {
                panic!("First poll result of epoch stream should be a `NewEpoch` event.");
            };
            (new_epoch_info, clock_tick, clock_stream)
        }
        .await;

        let membership_stream = MembershipAdapter::new(
            overwatch_handle
                .relay::<<MembershipAdapter as membership::Adapter>::Service>()
                .await?,
            settings
                .common
                .crypto
                .non_ephemeral_signing_key
                .public_key(),
            // We don't need to generate secret zk info in the proxy service, so we ignore the
            // secret key at this level.
            // TODO: Shouldn't use None, since this will be also used for core
            None,
        )
        .subscribe()
        .await
        .expect("subscribing to membership updates must succeed");

        let (current_membership_info, session_stream) = UninitializedSessionEventStream::new(
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
            current_membership_info.membership.size()
        );

        status_updater.notify_ready();
        info!(
            target: LOG_TARGET,
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        // TODO: Is it fine to wait for the first secret PoL info here?
        //
        // A Blend edge service without the required secret `PoL` to generate proofs for
        // block proposals info is useless, hence we wait until the first secret PoL
        // info is made available. If an edge node has very little to no stake, this
        // `await` might hang for a long time, but that is fine, since that means there
        // will be no blocks to blend anyway.
        let (current_private_leader_info, secret_pol_info_stream) = async {
            // There might be services that depend on Blend to be ready before starting, so
            // we cannot wait for the stream to be sent before we signal we are
            // ready, hence this should always be called after `notify_ready();`.
            // Also, Blend services start even if such a stream is not immediately
            // available, since they will simply keep blending cover messages.
            let mut secret_pol_info_stream = PolInfoProvider::subscribe(overwatch_handle)
                .await
                .expect("Should not fail to subscribe to secret PoL info stream.");
            (
                secret_pol_info_stream
                    .next()
                    .await
                    .expect("Secret PoL info stream should always return `Some` value."),
                secret_pol_info_stream,
            )
        }
        .await;

        let context = Context {
            service_message_stream: Box::pin(inbound_relay),
            current_membership_info,
            session_stream: session_stream.boxed().fork(),
            epoch_handler,
            current_leader_inputs_minus_quota,
            current_clock,
            clock_stream,
            current_private_leader_info,
            secret_pol_info_stream: secret_pol_info_stream.boxed(),
            overwatch_handle: overwatch_handle.clone(),
        };

        let mut mode: Box<
            dyn Mode<
                    Context<
                        <MembershipAdapter as membership::Adapter>::NodeId,
                        BroadcastSettings,
                        ChainService,
                        RuntimeServiceId,
                    >,
                > + Send,
        > = {
            if context.current_membership_info.membership.size() < minimal_network_size {
                todo!("Create BroadcastMode")
            } else if context.current_membership_info.membership.contains_local() {
                Box::new(
                    CoreMode::<
                        CoreBackend,
                        <MembershipAdapter as membership::Adapter>::NodeId,
                        NetworkAdapter,
                        MembershipAdapter,
                        CoreProofsGenerator,
                        ProofsVerifier,
                        ChainService,
                        PolInfoProvider,
                        RuntimeServiceId,
                    >::new(
                        context, settings.clone().into(), network_adapter.clone()
                    )
                    .expect("CoreMode must be created successfully."),
                )
            } else {
                Box::new(
                    EdgeMode::<
                        EdgeBackend,
                        <MembershipAdapter as membership::Adapter>::NodeId,
                        BroadcastSettings,
                        MembershipAdapter,
                        EdgeProofsGenerator,
                        ChainService,
                        PolInfoProvider,
                        RuntimeServiceId,
                    >::new(context, settings.clone().into())
                    .expect("EdgeMode must be created successfully."),
                )
            }
        };

        loop {
            match mode.run().await {
                Ok((next_mode_kind, context)) => {
                    mode = match next_mode_kind {
                        ModeKind::Core => Box::new(
                            CoreMode::<
                                CoreBackend,
                                <MembershipAdapter as membership::Adapter>::NodeId,
                                NetworkAdapter,
                                MembershipAdapter,
                                CoreProofsGenerator,
                                ProofsVerifier,
                                ChainService,
                                PolInfoProvider,
                                RuntimeServiceId,
                            >::new(
                                context, settings.clone().into(), network_adapter.clone()
                            )
                            .expect("CoreMode must be created successfully."),
                        ),
                        ModeKind::Edge => Box::new(
                            EdgeMode::<
                                EdgeBackend,
                                <MembershipAdapter as membership::Adapter>::NodeId,
                                BroadcastSettings,
                                MembershipAdapter,
                                EdgeProofsGenerator,
                                ChainService,
                                PolInfoProvider,
                                RuntimeServiceId,
                            >::new(context, settings.clone().into())
                            .expect("EdgeMode must be created successfully."),
                        ),
                        ModeKind::Broadcast => todo!("Create BroadcastMode"),
                    };
                }
                Err(e) => {
                    panic!("error when running mode: {e:?}")
                }
            }
        }
    }
}
