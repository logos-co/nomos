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

type MappingFuture = BoxFuture<'static, Result<Multiaddr, AddressMapperError>>;

/// Events emitted by the NAT address mapper
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Event {
    /// Address mapping failed for the given internal address
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
    /// No NAT mapping is currently active or in progress
    Idle,

    /// NAT mapping is being established or renewed
    Mapping {
        /// Internal address being mapped
        address: Multiaddr,
        /// Future for the mapping operation
        future: MappingFuture,
        /// Whether this is the initial mapping (vs renewal)
        is_initial: bool,
    },

    /// NAT mapping is active and being monitored for renewal
    Active {
        /// Internal address that is currently mapped
        internal: Multiaddr,
        /// Timer for when to renew the mapping
        renewal_timer: Pin<Box<Sleep>>,
    },
}

impl State {
    fn start_mapping<P: NatMapper>(
        address: Multiaddr,
        settings: NatMappingSettings,
        is_initial: bool,
    ) -> Self {
        debug!(
            %address,
            is_initial,
            "Starting NAT mapping"
        );

        let internal = address.clone();
        let future = async move {
            let mut mapper = P::initialize(settings).await?;
            mapper.map_address(&internal).await
        }
        .boxed();

        Self::Mapping {
            address,
            future,
            is_initial,
        }
    }

    fn active(internal: Multiaddr, lease_duration: u32) -> Self {
        let renewal_delay =
            Duration::from_secs_f64(f64::from(lease_duration) * RENEWAL_DELAY_FRACTION);

        Self::Active {
            internal,
            renewal_timer: Box::pin(time::sleep(renewal_delay)),
        }
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
            state: State::Idle,
            settings,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Attempts to map the given internal address to an external address
    ///
    /// Returns an error if mapping is already in progress for a different
    /// address. If the same address is already mapped, this is a no-op.
    pub fn try_map_address(&mut self, address: Multiaddr) -> Result<(), AddressMapperError> {
        match &self.state {
            State::Idle => {
                self.state = State::start_mapping::<P>(address, self.settings, true);
                Ok(())
            }
            State::Mapping { .. } => Err(AddressMapperError::MappingAlreadyInProgress),
            State::Active { internal, .. } => {
                if *internal == address {
                    return Ok(());
                }

                info!(
                    old = %internal,
                    new = %address,
                    "Replacing active mapping with new address"
                );

                self.state = State::start_mapping::<P>(address, self.settings, true);

                Ok(())
            }
        }
    }

    /// Polls the mapping future and handles completion or failure
    fn poll_mapping(
        &mut self,
        cx: &mut Context<'_>,
        address: Multiaddr,
        mut future: MappingFuture,
        is_initial: bool,
    ) -> Poll<ToSwarm<Event, Infallible>> {
        match future.poll_unpin(cx) {
            Poll::Ready(Ok(external)) => {
                info!(
                    %address,
                    %external,
                    "NAT mapping established"
                );

                self.state = State::active(address, self.settings.lease_duration);

                if is_initial {
                    Poll::Ready(ToSwarm::GenerateEvent(Event::NewExternalMappedAddress(
                        external,
                    )))
                } else {
                    Poll::Pending
                }
            }
            Poll::Ready(Err(error)) => {
                warn!(
                    address = %address,
                    error = %error,
                    "NAT mapping failed"
                );

                self.state = State::Idle;

                if is_initial {
                    Poll::Ready(ToSwarm::GenerateEvent(Event::AddressMappingFailed(address)))
                } else {
                    Poll::Pending
                }
            }
            Poll::Pending => {
                self.state = State::Mapping {
                    address,
                    future,
                    is_initial,
                };
                Poll::Pending
            }
        }
    }

    /// Polls the renewal timer
    fn poll_renewal(
        &mut self,
        cx: &mut Context<'_>,
        internal: Multiaddr,
        mut renewal_timer: Pin<Box<Sleep>>,
    ) -> Poll<ToSwarm<Event, Infallible>> {
        if renewal_timer.poll_unpin(cx).is_ready() {
            self.state = State::start_mapping::<P>(internal, self.settings, false);
        } else {
            self.state = State::Active {
                internal,
                renewal_timer,
            };
        }
        Poll::Pending
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
        let state = std::mem::replace(&mut self.state, State::Idle);

        match state {
            State::Idle => {
                self.state = State::Idle;
                Poll::Pending
            }
            State::Mapping {
                address,
                future,
                is_initial,
            } => self.poll_mapping(cx, address, future, is_initial),
            State::Active {
                internal,
                renewal_timer,
            } => self.poll_renewal(cx, internal, renewal_timer),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::future::poll_fn;

    use super::*;

    const EXTERNAL_ADDRESS: &str = "/ip4/203.0.113.1/tcp/12345";

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
            Ok(EXTERNAL_ADDRESS.parse().unwrap())
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

        let external_address: Multiaddr = EXTERNAL_ADDRESS.parse().unwrap();
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

        time::advance(Duration::from_millis(1900)).await;

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
            state: State::start_mapping::<MockMapper>(address1, behaviour.settings, true),
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
}
