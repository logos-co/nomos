mod errors;
mod nat_pmp;
pub mod protocol;
mod upnp;

use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{future::BoxFuture, FutureExt as _};
use libp2p::{
    core::{transport::PortUse, Endpoint, Multiaddr},
    swarm::{
        dummy::ConnectionHandler, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour,
        THandler, THandlerOutEvent, ToSwarm,
    },
    PeerId,
};
use tokio::time::{self, Sleep};
use tracing::{debug, info, warn};

use crate::{
    behaviour::nat::address_mapper::{errors::AddressMapperError, protocol::NatMapper},
    config::NatMappingSettings,
};

/// Renewal delay as a fraction of the lease duration
const RENEWAL_DELAY_FRACTION: f64 = 0.8;

/// Retry interval for failed mapping attempts
const RETRY_INTERVAL: Duration = Duration::from_secs(30);

type MappingFuture = BoxFuture<'static, Result<Multiaddr, AddressMapperError>>;

type PollResult = Poll<ToSwarm<Event, Infallible>>;

trait StateTrait {
    fn poll<P: NatMapper>(
        self,
        cx: &mut Context<'_>,
        settings: &NatMappingSettings,
    ) -> (PollResult, State);
}

/// Events emitted by the NAT address mapper
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Event {
    /// Address mapping failed for the given local address
    AddressMappingFailed(Multiaddr),
    /// The default gateway has changed
    DefaultGatewayChanged,
    /// The local address has changed
    LocalAddressChanged(Multiaddr),
    /// A new external address mapping has been successfully established
    NewExternalMappedAddress(Multiaddr),
}

/// Represents the current state of NAT address mapping
enum State {
    /// No NAT mapping is currently active or in progress.
    Idle(IdleState),

    /// NAT mapping is being established or renewed.
    Mapping(MappingState),

    /// NAT mapping is active and being monitored for renewal.
    Active(ActiveState),

    /// Waiting to retry after a failed mapping attempt.
    Retry(RetryState),
}

/// No NAT mapping is currently active or in progress.
#[derive(Debug)]
struct IdleState;

/// NAT mapping is being established or renewed.
struct MappingState {
    /// Local address being mapped.
    local_address: Multiaddr,
    /// Future for the mapping operation.
    future: MappingFuture,
    /// Whether this is the initial mapping (vs renewal).
    is_initial: bool,
    /// Number of retry attempts made.
    retry_count: u32,
}

/// NAT mapping is active and being monitored for renewal.
#[derive(Debug)]
struct ActiveState {
    /// Local address that is currently mapped.
    local_address: Multiaddr,
    /// Timer for when to renew the mapping.
    renewal_timer: Pin<Box<Sleep>>,
}

/// Waiting to retry after a failed mapping attempt.
#[derive(Debug)]
struct RetryState {
    /// Local address to retry mapping.
    local_address: Multiaddr,
    /// Timer for when to retry.
    retry_timer: Pin<Box<Sleep>>,
    /// Number of retry attempts made so far.
    retry_count: u32,
}

impl StateTrait for IdleState {
    fn poll<P: NatMapper>(
        self,
        _cx: &mut Context<'_>,
        _settings: &NatMappingSettings,
    ) -> (PollResult, State) {
        (Poll::Pending, State::Idle(self))
    }
}

impl StateTrait for MappingState {
    fn poll<P: NatMapper>(
        mut self,
        cx: &mut Context<'_>,
        settings: &NatMappingSettings,
    ) -> (PollResult, State) {
        match self.future.poll_unpin(cx) {
            Poll::Ready(Ok(external)) => Self::handle_success(self, external, settings),
            Poll::Ready(Err(error)) => Self::handle_failure(self, &error, settings),
            Poll::Pending => (Poll::Pending, State::Mapping(self)),
        }
    }
}

impl MappingState {
    fn handle_success(
        self,
        external: Multiaddr,
        settings: &NatMappingSettings,
    ) -> (PollResult, State) {
        info!(%self.local_address, %external, "NAT mapping established");

        let new_state = State::active(self.local_address, settings.lease_duration);

        let result = if self.is_initial {
            Poll::Ready(ToSwarm::GenerateEvent(Event::NewExternalMappedAddress(
                external,
            )))
        } else {
            Poll::Pending
        };

        (result, new_state)
    }

    fn handle_failure(
        self,
        error: &AddressMapperError,
        settings: &NatMappingSettings,
    ) -> (PollResult, State) {
        warn!(%self.local_address, %error, self.retry_count, "NAT mapping failed");

        if self.retry_count < settings.max_retries {
            (
                Poll::Pending,
                State::retry(
                    self.local_address,
                    self.retry_count,
                    Box::pin(time::sleep(RETRY_INTERVAL)),
                ),
            )
        } else {
            (
                Poll::Ready(ToSwarm::GenerateEvent(Event::AddressMappingFailed(
                    self.local_address,
                ))),
                State::Idle(IdleState),
            )
        }
    }
}

impl StateTrait for ActiveState {
    fn poll<P: NatMapper>(
        self,
        cx: &mut Context<'_>,
        settings: &NatMappingSettings,
    ) -> (PollResult, State) {
        let mut renewal_timer = self.renewal_timer;
        if renewal_timer.poll_unpin(cx).is_ready() {
            (
                Poll::Pending,
                State::mapping::<P>(self.local_address, *settings, false, 0),
            )
        } else {
            (
                Poll::Pending,
                State::active(self.local_address, settings.lease_duration),
            )
        }
    }
}

impl StateTrait for RetryState {
    fn poll<P: NatMapper>(
        self,
        cx: &mut Context<'_>,
        settings: &NatMappingSettings,
    ) -> (PollResult, State) {
        let mut retry_timer = self.retry_timer;
        if retry_timer.poll_unpin(cx).is_ready() {
            (
                Poll::Pending,
                State::mapping::<P>(self.local_address, *settings, false, self.retry_count + 1),
            )
        } else {
            (
                Poll::Pending,
                State::retry(self.local_address, self.retry_count, retry_timer),
            )
        }
    }
}

impl StateTrait for State {
    fn poll<P: NatMapper>(
        self,
        cx: &mut Context<'_>,
        settings: &NatMappingSettings,
    ) -> (PollResult, State) {
        match self {
            Self::Idle(state) => state.poll::<P>(cx, settings),
            Self::Mapping(state) => state.poll::<P>(cx, settings),
            Self::Active(state) => state.poll::<P>(cx, settings),
            Self::Retry(state) => state.poll::<P>(cx, settings),
        }
    }
}

impl State {
    fn mapping<P: NatMapper>(
        address: Multiaddr,
        settings: NatMappingSettings,
        is_initial: bool,
        retry_count: u32,
    ) -> Self {
        debug!(%address, is_initial, retry_count, "Starting NAT mapping");

        let local_address = address.clone();
        let future = async move {
            let mut mapper = P::initialize(settings).await?;
            mapper.map_address(&local_address).await
        }
        .boxed();

        Self::Mapping(MappingState {
            local_address: address,
            future,
            is_initial,
            retry_count,
        })
    }

    fn active(local_address: Multiaddr, lease_duration: u32) -> Self {
        let renewal_delay =
            Duration::from_secs_f64(f64::from(lease_duration) * RENEWAL_DELAY_FRACTION);

        Self::Active(ActiveState {
            local_address,
            renewal_timer: Box::pin(time::sleep(renewal_delay)),
        })
    }

    const fn retry(
        local_address: Multiaddr,
        retry_count: u32,
        retry_timer: Pin<Box<Sleep>>,
    ) -> Self {
        Self::Retry(RetryState {
            local_address,
            retry_timer,
            retry_count,
        })
    }
}

/// Network behaviour for managing NAT address mapping
pub struct AddressMapperBehaviour<P> {
    /// Current state of the NAT mapping
    state: State,
    /// Configuration settings for NAT mapping
    settings: NatMappingSettings,
    _phantom: std::marker::PhantomData<P>,
}

impl<P> AddressMapperBehaviour<P>
where
    P: NatMapper,
{
    /// Creates a new address mapper behaviour with the given settings
    pub const fn new(settings: NatMappingSettings) -> Self {
        Self {
            state: State::Idle(IdleState),
            settings,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Attempts to map the given local address to an external address
    ///
    /// Returns an error if mapping is already in progress for a different
    /// address. If the same address is already mapped, this is a no-op.
    pub fn try_map_address(&mut self, address: Multiaddr) -> Result<(), AddressMapperError> {
        match &self.state {
            State::Idle(_) => {
                self.state = State::mapping::<P>(address, self.settings, true, 0);
                Ok(())
            }
            State::Mapping(_) | State::Retry(_) => {
                Err(AddressMapperError::MappingAlreadyInProgress)
            }
            State::Active(active_state) => {
                if active_state.local_address == address {
                    return Ok(());
                }

                info!(old = %active_state.local_address, new = %address, "Replacing active mapping with new address");
                self.state = State::mapping::<P>(address, self.settings, true, 0);

                Ok(())
            }
        }
    }
}

impl<P> NetworkBehaviour for AddressMapperBehaviour<P>
where
    P: NatMapper,
{
    type ConnectionHandler = ConnectionHandler;
    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(ConnectionHandler)
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(ConnectionHandler)
    }

    fn on_swarm_event(&mut self, _: FromSwarm) {}

    fn on_connection_handler_event(
        &mut self,
        _: PeerId,
        _: ConnectionId,
        _: THandlerOutEvent<Self>,
    ) {
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ToSwarm<Event, Infallible>> {
        let state = std::mem::replace(&mut self.state, State::Idle(IdleState));

        let (poll_result, new_state) = state.poll::<P>(cx, &self.settings);

        self.state = new_state;

        poll_result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::future::poll_fn;

    use super::*;

    const MOCK_EXTERNAL_ADDRESS: &str = "/ip4/203.0.113.1/tcp/12345";

    thread_local! {
        static CALL_COUNT: AtomicUsize = const { AtomicUsize::new(0) };
    }

    struct MockMapper;

    impl MockMapper {
        fn reset_mapping_attempts_count() {
            CALL_COUNT.with(|c| c.store(0, Ordering::SeqCst));
        }

        fn get_mapping_attempts_count() -> usize {
            CALL_COUNT.with(|c| c.load(Ordering::SeqCst))
        }
    }

    struct FailingMockMapper;

    impl FailingMockMapper {
        fn reset_mapping_attempts_count() {
            CALL_COUNT.with(|c| c.store(0, Ordering::SeqCst));
        }

        fn get_mapping_attempts_count() -> usize {
            CALL_COUNT.with(|c| c.load(Ordering::SeqCst))
        }
    }

    #[async_trait::async_trait]
    impl NatMapper for MockMapper {
        async fn initialize(_settings: NatMappingSettings) -> Result<Box<Self>, AddressMapperError>
        where
            Self: Sized,
        {
            Ok(Box::new(Self))
        }

        async fn map_address(
            &mut self,
            _address: &Multiaddr,
        ) -> Result<Multiaddr, AddressMapperError> {
            CALL_COUNT.with(|c| c.fetch_add(1, Ordering::SeqCst));
            Ok(MOCK_EXTERNAL_ADDRESS.parse().unwrap())
        }
    }

    #[async_trait::async_trait]
    impl NatMapper for FailingMockMapper {
        async fn initialize(_settings: NatMappingSettings) -> Result<Box<Self>, AddressMapperError>
        where
            Self: Sized,
        {
            Ok(Box::new(Self))
        }

        async fn map_address(
            &mut self,
            _address: &Multiaddr,
        ) -> Result<Multiaddr, AddressMapperError> {
            CALL_COUNT.with(|c| c.fetch_add(1, Ordering::SeqCst));
            Err(AddressMapperError::PortMappingFailed(
                "Mock mapping failure".into(),
            ))
        }
    }

    async fn poll_until_active<P: NatMapper>(behaviour: &mut AddressMapperBehaviour<P>) -> bool {
        time::timeout(Duration::from_secs(1), async {
            loop {
                poll_fn(|cx| {
                    let _ = behaviour.poll(cx);
                    Poll::Ready(())
                })
                .await;

                if matches!(behaviour.state, State::Active { .. }) {
                    return;
                }

                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok()
    }

    #[tokio::test]
    async fn test_successful_mapping() {
        let mut behaviour =
            AddressMapperBehaviour::<MockMapper>::new(NatMappingSettings::default());

        let address: Multiaddr = "/ip4/192.168.1.100/tcp/8080".parse().unwrap();

        behaviour.try_map_address(address).unwrap();

        let event = poll_fn(|cx| match behaviour.poll(cx) {
            Poll::Ready(ToSwarm::GenerateEvent(e)) => Poll::Ready(Some(e)),
            _ => Poll::Ready(None),
        })
        .await;

        let external_address: Multiaddr = MOCK_EXTERNAL_ADDRESS.parse().unwrap();
        assert!(
            matches!(event, Some(Event::NewExternalMappedAddress(addr)) if addr == external_address)
        );
    }

    #[tokio::test]
    async fn test_renewal() {
        time::pause();
        MockMapper::reset_mapping_attempts_count();

        let settings = NatMappingSettings {
            lease_duration: 2,
            ..Default::default()
        };

        let mut behaviour = AddressMapperBehaviour::<MockMapper>::new(settings);
        let address: Multiaddr = "/ip4/192.168.1.100/tcp/8080".parse().unwrap();

        behaviour.try_map_address(address).unwrap();

        assert!(poll_until_active(&mut behaviour).await);
        assert_eq!(MockMapper::get_mapping_attempts_count(), 1);

        time::advance(Duration::from_millis(1700)).await;

        poll_fn(|cx| {
            let _ = behaviour.poll(cx);
            Poll::Ready(())
        })
        .await;

        assert!(matches!(behaviour.state, State::Mapping { .. }));

        assert!(poll_until_active(&mut behaviour).await);
        assert_eq!(MockMapper::get_mapping_attempts_count(), 2);
    }

    #[tokio::test]
    async fn test_cant_map_while_mapping() {
        let behaviour = AddressMapperBehaviour::<MockMapper>::new(NatMappingSettings::default());
        let address1: Multiaddr = "/ip4/192.168.1.100/tcp/8080".parse().unwrap();
        let address2: Multiaddr = "/ip4/192.168.1.101/tcp/8081".parse().unwrap();

        let mut behaviour = AddressMapperBehaviour {
            state: State::mapping::<MockMapper>(address1, behaviour.settings, true, 0),
            ..behaviour
        };

        let result = behaviour.try_map_address(address2);
        assert!(matches!(
            result,
            Err(AddressMapperError::MappingAlreadyInProgress)
        ));
    }

    #[tokio::test]
    async fn test_replace_active_mapping() {
        MockMapper::reset_mapping_attempts_count();

        let mut behaviour =
            AddressMapperBehaviour::<MockMapper>::new(NatMappingSettings::default());

        let address1: Multiaddr = "/ip4/192.168.1.100/tcp/8080".parse().unwrap();
        let address2: Multiaddr = "/ip4/192.168.1.101/tcp/8081".parse().unwrap();

        behaviour.try_map_address(address1).unwrap();

        assert!(poll_until_active(&mut behaviour).await);
        assert_eq!(MockMapper::get_mapping_attempts_count(), 1);

        behaviour.try_map_address(address2.clone()).unwrap();

        let event = poll_fn(|cx| match behaviour.poll(cx) {
            Poll::Ready(ToSwarm::GenerateEvent(e)) => Poll::Ready(Some(e)),
            _ => Poll::Ready(None),
        })
        .await;

        assert!(matches!(event, Some(Event::NewExternalMappedAddress(_))));
        assert_eq!(MockMapper::get_mapping_attempts_count(), 2);
    }

    #[tokio::test]
    async fn test_same_address_while_active() {
        MockMapper::reset_mapping_attempts_count();

        let mut behaviour =
            AddressMapperBehaviour::<MockMapper>::new(NatMappingSettings::default());
        let address: Multiaddr = "/ip4/192.168.1.100/tcp/8080".parse().unwrap();

        behaviour.try_map_address(address.clone()).unwrap();

        assert!(poll_until_active(&mut behaviour).await);
        assert_eq!(MockMapper::get_mapping_attempts_count(), 1);

        let result = behaviour.try_map_address(address);
        assert!(result.is_ok());

        poll_fn(|cx| {
            let _ = behaviour.poll(cx);
            Poll::Ready(())
        })
        .await;

        assert_eq!(MockMapper::get_mapping_attempts_count(), 1);
    }

    #[tokio::test]
    async fn test_retry_full_cycle() {
        time::pause();
        FailingMockMapper::reset_mapping_attempts_count();

        let settings = NatMappingSettings {
            max_retries: 2,
            ..Default::default()
        };

        let mut behaviour = AddressMapperBehaviour::<FailingMockMapper>::new(settings);
        let address: Multiaddr = "/ip4/192.168.1.100/tcp/8080".parse().unwrap();

        behaviour.try_map_address(address.clone()).unwrap();

        let event = time::timeout(Duration::from_secs(200), async {
            loop {
                if let Some(e) = poll_fn(|cx| match behaviour.poll(cx) {
                    Poll::Ready(ToSwarm::GenerateEvent(e)) => Poll::Ready(Some(e)),
                    _ => Poll::Ready(None),
                })
                .await
                {
                    return e;
                }
                time::advance(Duration::from_secs(35)).await;
            }
        })
        .await
        .expect("Should receive failure event within timeout");

        assert!(matches!(event, Event::AddressMappingFailed(addr) if addr == address));

        let attempt_count = FailingMockMapper::get_mapping_attempts_count();
        assert!(attempt_count >= 3);

        assert!(matches!(behaviour.state, State::Idle(_)));
    }
}
