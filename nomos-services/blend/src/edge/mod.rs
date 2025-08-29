pub mod backends;
pub(crate) mod service_components;
pub mod settings;

use std::{
    fmt::{Debug, Display},
    hash::Hash,
    marker::PhantomData,
    time::Duration,
};

use backends::BlendBackend;
use futures::{Stream, StreamExt as _};
use nomos_blend_scheduling::{
    membership::Membership,
    message_blend::crypto::CryptographicProcessor,
    session::{SessionEvent, SessionEventStream},
};
use nomos_core::wire;
use nomos_utils::blake_rng::BlakeRng;
use overwatch::{
    overwatch::OverwatchHandle,
    services::{
        resources::ServiceResourcesHandle,
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceResourcesHandle,
};
use rand::SeedableRng as _;
use serde::Serialize;
pub(crate) use service_components::ServiceComponents;
use services_utils::wait_until_services_are_ready;
use settings::BlendConfig;
use tracing::{debug, warn};

use crate::{membership, message::ServiceMessage, settings::constant_membership_stream};

const LOG_TARGET: &str = "blend::service::edge";

pub struct BlendService<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    _phantom: PhantomData<MembershipAdapter>,
}

impl<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId> ServiceData
    for BlendService<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone,
{
    type Settings = BlendConfig<Backend::Settings, NodeId>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ServiceMessage<BroadcastSettings>;
}

#[async_trait::async_trait]
impl<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId>
    ServiceCore<RuntimeServiceId>
    for BlendService<Backend, NodeId, BroadcastSettings, MembershipAdapter, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Send + Sync,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    BroadcastSettings: Serialize + Send,
    MembershipAdapter: membership::Adapter + Send,
    membership::ServiceMessage<MembershipAdapter>: Send + Sync + 'static,
    <MembershipAdapter as membership::Adapter>::Error: Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<<MembershipAdapter as membership::Adapter>::Service>
        + AsServiceId<Self>
        + Display
        + Debug
        + Clone
        + Send
        + Sync
        + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            service_resources_handle,
            _phantom: PhantomData,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            service_resources_handle:
                ServiceResourcesHandle {
                    inbound_relay,
                    overwatch_handle,
                    settings_handle,
                    ..
                },
            ..
        } = self;

        let settings = settings_handle.notifier().get_updated_settings();
        let membership = settings.membership();

        let _membership_stream = MembershipAdapter::new(
            overwatch_handle
                .relay::<<MembershipAdapter as membership::Adapter>::Service>()
                .await?,
            settings.crypto.signing_private_key.public_key(),
        )
        .subscribe()
        .await?;
        // TODO: Use membership_stream once the membership/SDP services are ready to provide the real membership: https://github.com/logos-co/nomos/issues/1532

        let session_stream = SessionEventStream::new(
            Box::pin(constant_membership_stream(
                membership.clone(),
                settings.time.session_duration(),
            )),
            settings.time.session_transition_period(),
        );

        let messages_to_blend = inbound_relay.map(|ServiceMessage::Blend(message)| {
            wire::serialize(&message)
                .expect("Message from internal services should not fail to serialize")
        });

        wait_until_services_are_ready!(
            &overwatch_handle,
            Some(Duration::from_secs(60)),
            <MembershipAdapter as membership::Adapter>::Service
        )
        .await?;

        run::<Backend, _, _>(
            membership,
            session_stream,
            messages_to_blend,
            &settings,
            &overwatch_handle,
        )
        .await
    }
}

/// Run the main loop of the service.
///
/// It listens for new sessions and messages to blend, and handles them
/// accordingly.
///
/// It recreates the [`MessageHandler`] on each new session.
///
/// If the membership is smaller than the minimum network size, it drops and
/// doesn't recreate the [`MessageHandler`]. Inbound messages are ignored in
/// this case.
async fn run<Backend, NodeId, RuntimeServiceId>(
    initial_membership: Membership<NodeId>,
    mut session_stream: impl Stream<Item = SessionEvent<Membership<NodeId>>> + Send + Unpin,
    mut messages_to_blend: impl Stream<Item = Vec<u8>> + Send + Unpin,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
) -> Result<(), overwatch::DynError>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Sync,
    NodeId: Clone + Eq + Hash + Send + Sync + 'static,
    RuntimeServiceId: Clone,
{
    let mut message_handler = None;
    if initial_membership.size() >= settings.minimum_network_size.get() as usize {
        message_handler = Some(MessageHandler::<Backend, NodeId, RuntimeServiceId>::new(
            settings,
            initial_membership,
            overwatch_handle.clone(),
        ));
    }

    loop {
        tokio::select! {
            Some(SessionEvent::NewSession(membership)) = session_stream.next() => {
                message_handler = handle_new_session(membership, settings, overwatch_handle);
            }
            Some(message) = messages_to_blend.next() => {
                if let Some(message_handler) = message_handler.as_mut() {
                    message_handler.handle_messages_to_blend(message).await;
                } else {
                    warn!(target: LOG_TARGET, "Ignoring message as the blend network is too small");
                }
            }
        }
    }
}

/// Handle a new session.
///
/// It creates a new [`MessageHandler`] if the membership is larger than the
/// minimum network size. Otherwise, it returns [`None`].
fn handle_new_session<Backend, NodeId, RuntimeServiceId>(
    membership: Membership<NodeId>,
    settings: &Settings<Backend, NodeId, RuntimeServiceId>,
    overwatch_handle: &OverwatchHandle<RuntimeServiceId>,
) -> Option<MessageHandler<Backend, NodeId, RuntimeServiceId>>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone + Send + 'static,
    RuntimeServiceId: Clone,
{
    if membership.size() < settings.minimum_network_size.get() as usize {
        warn!(target: LOG_TARGET, "Not creating a new message handler as the blend network is too small");
        return None;
    }

    debug!(target: LOG_TARGET, "Creating a new message handler");
    Some(MessageHandler::<Backend, NodeId, RuntimeServiceId>::new(
        settings,
        membership,
        overwatch_handle.clone(),
    ))
}

type Settings<Backend, NodeId, RuntimeServiceId> =
    BlendConfig<<Backend as BlendBackend<NodeId, RuntimeServiceId>>::Settings, NodeId>;

struct MessageHandler<Backend, NodeId, RuntimeServiceId> {
    cryptographic_processor: CryptographicProcessor<NodeId, BlakeRng>,
    backend: Backend,
    _phantom: PhantomData<RuntimeServiceId>,
}

impl<Backend, NodeId, RuntimeServiceId> MessageHandler<Backend, NodeId, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, RuntimeServiceId>,
    NodeId: Clone + Send + 'static,
{
    fn new(
        settings: &Settings<Backend, NodeId, RuntimeServiceId>,
        membership: Membership<NodeId>,
        overwatch_handle: OverwatchHandle<RuntimeServiceId>,
    ) -> Self {
        let cryptographic_processor = CryptographicProcessor::new(
            settings.crypto.clone(),
            membership.clone(),
            BlakeRng::from_entropy(),
        );
        let backend = Backend::new(
            settings.backend.clone(),
            overwatch_handle,
            membership,
            BlakeRng::from_entropy(),
        );
        Self {
            cryptographic_processor,
            backend,
            _phantom: PhantomData,
        }
    }
}

impl<Backend, NodeId, RuntimeServiceId> MessageHandler<Backend, NodeId, RuntimeServiceId>
where
    NodeId: Eq + Hash + Clone + Send,
    Backend: BlendBackend<NodeId, RuntimeServiceId> + Sync,
{
    /// Blend a new message received from another service.
    async fn handle_messages_to_blend(&mut self, message: Vec<u8>) {
        let Ok(message) = self
            .cryptographic_processor
            .encapsulate_data_payload(&message)
            .inspect_err(|e| {
                tracing::error!(target: LOG_TARGET, "Failed to encapsulate message: {e:?}");
            })
        else {
            return;
        };
        self.backend.send(message).await;
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use libp2p::Multiaddr;
    use nomos_blend_message::crypto::{Ed25519PrivateKey, Ed25519PublicKey};
    use nomos_blend_scheduling::{
        membership::Node, message_blend::CryptographicProcessorSettings, EncapsulatedMessage,
    };
    use overwatch::overwatch::commands::OverwatchCommand;
    use rand::{rngs::OsRng, RngCore};
    use tokio::{
        sync::mpsc::{self, error::TryRecvError},
        time::sleep,
    };
    use tokio_stream::wrappers::ReceiverStream;

    use super::*;
    use crate::settings::TimingSettings;

    #[test_log::test(tokio::test)]
    async fn run_with_session_transition() {
        let (session_sender, session_receiver) = mpsc::channel(1);
        let (msg_sender, msg_receiver) = mpsc::channel(1);
        let (node_id_sender, mut node_id_receiver) = mpsc::channel(1);

        let mut core_node = 0;
        let minimum_network_size = 1;
        tokio::spawn(async move {
            let _ = run::<TestBackend, _, _>(
                membership(&[core_node], None),
                ReceiverStream::new(session_receiver),
                ReceiverStream::new(msg_receiver),
                &settings(10, minimum_network_size, node_id_sender),
                &overwatch_handle(),
            )
            .await;
        });

        // A message should be forwarded to the core node 0.
        msg_sender.send(vec![0]).await.expect("channel opened");
        assert_eq!(
            node_id_receiver.recv().await.expect("channel opened"),
            core_node
        );

        // Send a new session with another core node 1.
        core_node = 1;
        session_sender
            .send(SessionEvent::NewSession(membership(&[core_node], None)))
            .await
            .expect("channel opened");
        sleep(Duration::from_millis(100)).await;

        // A message should be forwarded to the core node 1.
        msg_sender.send(vec![0]).await.expect("channel opened");
        assert_eq!(
            node_id_receiver.recv().await.expect("channel opened"),
            core_node
        );

        // Send a TransitionPeriodExpired that should be ignored.
        session_sender
            .send(SessionEvent::TransitionPeriodExpired)
            .await
            .expect("channel opened");
        sleep(Duration::from_millis(100)).await;
        msg_sender.send(vec![0]).await.expect("channel opened");
        assert_eq!(
            node_id_receiver.recv().await.expect("channel opened"),
            core_node
        );

        // Send a new session with an empty membership (smaller than the min size).
        session_sender
            .send(SessionEvent::NewSession(membership(&[], None)))
            .await
            .expect("channel opened");
        sleep(Duration::from_millis(100)).await;

        // A message should be ignored.
        msg_sender.send(vec![0]).await.expect("channel opened");
        sleep(Duration::from_millis(100)).await;
        assert!(matches!(
            node_id_receiver.try_recv(),
            Err(TryRecvError::Empty)
        ));
    }

    #[test_log::test(tokio::test)]
    async fn run_with_small_initial_membership() {
        let (session_sender, session_receiver) = mpsc::channel(1);
        let (msg_sender, msg_receiver) = mpsc::channel(1);
        let (node_id_sender, mut node_id_receiver) = mpsc::channel(1);

        let core_nodes = [0];
        let minimum_network_size = 2;
        tokio::spawn(async move {
            let _ = run::<TestBackend, _, _>(
                membership(&core_nodes, None),
                ReceiverStream::new(session_receiver),
                ReceiverStream::new(msg_receiver),
                &settings(10, minimum_network_size, node_id_sender),
                &overwatch_handle(),
            )
            .await;
        });

        // A message should be ignored as the network is small.
        msg_sender.send(vec![0]).await.expect("channel opened");
        sleep(Duration::from_millis(100)).await;
        assert!(matches!(
            node_id_receiver.try_recv(),
            Err(TryRecvError::Empty)
        ));

        // Send a new session with more nodes.
        let core_nodes = [0, 1];
        session_sender
            .send(SessionEvent::NewSession(membership(&core_nodes, None)))
            .await
            .expect("channel opened");
        sleep(Duration::from_millis(100)).await;

        // A message should be forwarded to one of the core nodes.
        msg_sender.send(vec![0]).await.expect("channel opened");
        assert!(core_nodes.contains(&node_id_receiver.recv().await.expect("channel opened")));
    }

    fn membership(ids: &[NodeId], local_id: Option<NodeId>) -> Membership<NodeId> {
        Membership::new(
            &ids.iter()
                .map(|id| Node {
                    id: *id,
                    address: Multiaddr::empty(),
                    public_key: key(*id).1,
                })
                .collect::<Vec<_>>(),
            local_id.map(|id| key(id).1).as_ref(),
        )
    }

    fn key(id: NodeId) -> (Ed25519PrivateKey, Ed25519PublicKey) {
        let private_key = Ed25519PrivateKey::from([id; 32]);
        let public_key = private_key.public_key();
        (private_key, public_key)
    }

    fn settings(
        local_id: NodeId,
        minimum_network_size: u64,
        msg_sender: NodeIdSender,
    ) -> BlendConfig<NodeIdSender, NodeId> {
        BlendConfig {
            membership: Vec::new(),
            time: TimingSettings {
                rounds_per_session: NonZeroU64::new(1).unwrap(),
                rounds_per_interval: NonZeroU64::new(1).unwrap(),
                round_duration: Duration::from_secs(1),
                rounds_per_observation_window: NonZeroU64::new(1).unwrap(),
                rounds_per_session_transition_period: NonZeroU64::new(1).unwrap(),
            },
            crypto: CryptographicProcessorSettings {
                signing_private_key: key(local_id).0,
                num_blend_layers: 1,
            },
            backend: msg_sender,
            minimum_network_size: NonZeroU64::new(minimum_network_size).unwrap(),
        }
    }

    type NodeId = u8;
    type NodeIdSender = mpsc::Sender<NodeId>;

    struct TestBackend {
        membership: Membership<NodeId>,
        sender: NodeIdSender,
    }

    #[async_trait::async_trait]
    impl<RuntimeServiceId> BlendBackend<NodeId, RuntimeServiceId> for TestBackend
    where
        NodeId: Clone,
        RuntimeServiceId: Debug + Sync + Display,
    {
        type Settings = NodeIdSender;

        fn new<Rng>(
            settings: Self::Settings,
            _: OverwatchHandle<RuntimeServiceId>,
            membership: Membership<NodeId>,
            _: Rng,
        ) -> Self
        where
            Rng: RngCore + Send + 'static,
        {
            Self {
                membership,
                sender: settings,
            }
        }

        fn shutdown(self) {}

        async fn send(&self, _: EncapsulatedMessage) {
            let node_id = self
                .membership
                .choose_remote_nodes(&mut OsRng, 1)
                .next()
                .expect("Membership should not be empty")
                .id;
            self.sender.send(node_id).await.unwrap();
        }
    }

    fn overwatch_handle() -> OverwatchHandle<usize> {
        let (sender, _) = mpsc::channel::<OverwatchCommand<usize>>(1);
        OverwatchHandle::new(tokio::runtime::Handle::current(), sender)
    }
}
