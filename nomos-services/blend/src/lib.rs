use std::{
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
    time::Duration,
};

use async_trait::async_trait;
use futures::{Stream, StreamExt as _};
pub use nomos_blend_message::{crypto::proofs::RealProofsVerifier, encap::ProofsVerifier};
pub use nomos_blend_scheduling::message_blend::{ProofsGenerator, RealProofsGenerator};
use nomos_blend_scheduling::{
    message_blend::{PrivateInputs, PublicInputs},
    session::UninitializedSessionEventStream,
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
        network::NetworkAdapter as NetworkAdapterTrait,
        service_components::{
            MessageComponents, NetworkBackendOfService, ServiceComponents as CoreServiceComponents,
        },
    },
    instance::{Instance, Mode},
    membership::Adapter as _,
    settings::{FIRST_SESSION_READY_TIMEOUT, Settings},
};

pub mod core;
pub mod edge;
pub mod membership;
pub mod message;
pub mod session;
pub mod settings;

mod instance;
mod modes;
mod service_components;
pub use self::service_components::ServiceComponents;

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
    type Settings = Settings;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = CoreService::Message;
}

#[async_trait]
impl<CoreService, EdgeService, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlendService<CoreService, EdgeService, RuntimeServiceId>
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
    EdgeService: ServiceData<Message = CoreService::Message>
        // We tie the core and edge proofs generator to be the same type, to avoid mistakes in the
        // node configuration where the two services use different verification logic
        + edge::ServiceComponents<ProofsGenerator = CoreService::ProofsGenerator>
        + Send
        + 'static,
    EdgeService::MembershipAdapter:
        membership::Adapter<NodeId = CoreService::NodeId, Error: Send + Sync + 'static> + Send,
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

        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(30)),
            MembershipService<EdgeService>
        )
        .await?;

        let membership_stream = <MembershipAdapter<EdgeService> as membership::Adapter>::new(
            overwatch_handle
                .relay::<MembershipService<EdgeService>>()
                .await?,
            settings.crypto.non_ephemeral_signing_key.public_key(),
        )
        .subscribe()
        .await?;

        let (membership, mut session_stream) = UninitializedSessionEventStream::new(
            membership_stream,
            FIRST_SESSION_READY_TIMEOUT,
            settings.time.session_transition_period(),
        )
        .await_first_ready()
        .await
        .expect("The current session must be ready");

        info!(
            target: LOG_TARGET,
            "The current membership is ready: {} nodes.",
            membership.size()
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

type MembershipAdapter<EdgeService> = <EdgeService as edge::ServiceComponents>::MembershipAdapter;

type MembershipService<EdgeService> =
    <MembershipAdapter<EdgeService> as membership::Adapter>::Service;

const fn mock_poq_inputs() -> (PublicInputs, PrivateInputs) {
    use groth16::Field as _;
    use nomos_core::crypto::ZkHash;

    (
        PublicInputs {
            core_quota: 0,
            core_root: ZkHash::ZERO,
            leader_quota: 0,
            pol_epoch_nonce: ZkHash::ZERO,
            pol_ledger_aged: ZkHash::ZERO,
            session: 0,
            total_stake: 0,
        },
        PrivateInputs {
            aged_path: vec![],
            aged_selector: vec![],
            core_path: vec![],
            core_path_selectors: vec![],
            core_sk: ZkHash::ZERO,
            note_value: 0,
            output_number: 0,
            pol_secret_key: ZkHash::ZERO,
            slot: 0,
            slot_secret: ZkHash::ZERO,
            slot_secret_path: vec![],
            starting_slot: 0,
            transaction_hash: ZkHash::ZERO,
        },
    )
}

fn mock_poq_inputs_stream() -> impl Stream<Item = (PublicInputs, PrivateInputs)> {
    use futures::stream::repeat;

    repeat(mock_poq_inputs())
}
