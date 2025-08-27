use std::{
    task::{Context, Poll},
    time::Duration,
};

use either::Either;
use futures::{
    future::{BoxFuture, Fuse, OptionFuture},
    FutureExt as _,
};
use libp2p::{
    autonat,
    core::{transport::PortUse, Endpoint},
    swarm::{
        ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent,
        THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};
use rand::RngCore;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, info};

use crate::{
    behaviour::nat::{
        address_mapper,
        address_mapper::{protocols::ProtocolManager, AddressMapperBehaviour, NatMapper},
        gateway_monitor::{
            GatewayDetector, GatewayMonitor, GatewayMonitorEvent, SystemGatewayDetector,
        },
        state_machine::{Command, StateMachine},
    },
    config::NatSettings,
};

type Task = BoxFuture<'static, Multiaddr>;

pub struct InnerNatBehaviour<R, Mapper, Detector>
where
    R: RngCore + 'static,
{
    /// `AutoNAT` client behaviour which is used to confirm if addresses of our
    /// node are indeed publicly reachable.
    autonat_client_behaviour: autonat::v2::client::Behaviour<R>,
    /// The address mapper behaviour is used to map the node's addresses at the
    /// default gateway using one of the protocols: `PCP`, `NAT-PMP`,
    /// `UPNP-IGD`.
    address_mapper_behaviour: AddressMapperBehaviour<Mapper>,
    /// Gateway monitor that periodically checks for gateway address changes
    /// and triggers re-mapping when the gateway changes.
    gateway_monitor: GatewayMonitor<Detector>,
    /// The state machine reacts to events from the swarm and from the
    /// sub-behaviours of the `InnerNatBehaviour` and issues commands to the
    /// `InnerNatBehaviour`.
    state_machine: StateMachine,
    /// Commands issued by the state machine are received through this end of
    /// the channel.
    command_rx: UnboundedReceiver<Command>,
    /// Used to schedule "re-tests" for already confirmed external addresses via
    /// the `autonat_client_behaviour`. Unused outside of the states that
    /// require periodic maintenance.
    next_autonat_client_tick: Fuse<OptionFuture<Task>>,
    /// Interval for the above ticker.
    autonat_client_tick_interval: Duration,
    /// Current local address that is being managed
    local_address: Option<Multiaddr>,
}

pub type NatBehaviour<R> = InnerNatBehaviour<R, ProtocolManager, SystemGatewayDetector>;

impl<R: RngCore + 'static> NatBehaviour<R> {
    pub fn new(rng: R, nat_config: NatSettings) -> Self {
        let address_mapper_behaviour =
            AddressMapperBehaviour::<ProtocolManager>::new(nat_config.mapping);

        let gateway_monitor =
            GatewayMonitor::<SystemGatewayDetector>::new(nat_config.gateway_monitor);

        Self::create(rng, nat_config, address_mapper_behaviour, gateway_monitor)
    }
}

impl<R, Mapper, Detector> InnerNatBehaviour<R, Mapper, Detector>
where
    R: RngCore + 'static,
{
    fn create(
        rng: R,
        nat_config: NatSettings,
        address_mapper_behaviour: AddressMapperBehaviour<Mapper>,
        gateway_monitor: GatewayMonitor<Detector>,
    ) -> Self {
        let autonat_client_behaviour =
            autonat::v2::client::Behaviour::new(rng, nat_config.autonat.to_libp2p_config());

        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let state_machine = StateMachine::new(command_tx);

        let autonat_client_tick_interval = Duration::from_millis(
            nat_config
                .autonat
                .retest_successful_external_addresses_interval_millisecs,
        );

        Self {
            autonat_client_behaviour,
            address_mapper_behaviour,
            gateway_monitor,
            state_machine,
            command_rx,
            next_autonat_client_tick: OptionFuture::default().fuse(),
            autonat_client_tick_interval,
            local_address: None,
        }
    }
}

impl<R, Mapper, Detector> NetworkBehaviour for InnerNatBehaviour<R, Mapper, Detector>
where
    R: RngCore + 'static,
    Mapper: NatMapper + 'static,
    Detector: GatewayDetector + 'static,
{
    type ConnectionHandler =
        <autonat::v2::client::Behaviour as NetworkBehaviour>::ConnectionHandler;

    type ToSwarm = Either<
        <autonat::v2::client::Behaviour as NetworkBehaviour>::ToSwarm,
        address_mapper::Event,
    >;

    fn handle_pending_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        self.autonat_client_behaviour
            .handle_pending_inbound_connection(connection_id, local_addr, remote_addr)
    }

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.autonat_client_behaviour
            .handle_established_inbound_connection(connection_id, peer, local_addr, remote_addr)
    }

    fn handle_pending_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        maybe_peer: Option<PeerId>,
        addresses: &[Multiaddr],
        effective_role: Endpoint,
    ) -> Result<Vec<Multiaddr>, ConnectionDenied> {
        self.autonat_client_behaviour
            .handle_pending_outbound_connection(
                connection_id,
                maybe_peer,
                addresses,
                effective_role,
            )
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        role_override: Endpoint,
        port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.autonat_client_behaviour
            .handle_established_outbound_connection(
                connection_id,
                peer,
                addr,
                role_override,
                port_use,
            )
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        self.state_machine.on_event(event);
        self.autonat_client_behaviour.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        self.autonat_client_behaviour
            .on_connection_handler_event(peer_id, connection_id, event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        if let Poll::Ready(to_swarm) = self.autonat_client_behaviour.poll(cx) {
            if let ToSwarm::GenerateEvent(event) = &to_swarm {
                self.state_machine.on_event(event);
            }

            return Poll::Ready(to_swarm.map_out(Either::Left));
        }

        if let Poll::Ready(to_swarm) = self.address_mapper_behaviour.poll(cx) {
            if let ToSwarm::GenerateEvent(event) = &to_swarm {
                self.state_machine.on_event(event);
            }

            return Poll::Ready(to_swarm.map_out(Either::Right).map_in(Either::Right));
        }

        if let Poll::Ready(Some(_addr)) = self.next_autonat_client_tick.poll_unpin(cx) {
            // TODO: This is a placeholder for the missing API of the
            // autonat client
            // self.autonat_client_behaviour.retest_address(addr);
        }

        if let Poll::Ready(Some(event)) = self.gateway_monitor.poll(cx) {
            match event {
                GatewayMonitorEvent::GatewayChanged {
                    old_gateway,
                    new_gateway,
                } => {
                    info!(
                        "Gateway changed from {old_gateway:?} to {new_gateway:?}, triggering address re-mapping",
                    );

                    self.state_machine
                        .on_event(&address_mapper::Event::DefaultGatewayChanged {
                            old_gateway,
                            new_gateway,
                            local_address: self.local_address.clone(),
                        });
                }
            }
        }

        if let Poll::Ready(Some(command)) = self.command_rx.poll_recv(cx) {
            match command {
                Command::ScheduleAutonatClientTest(addr) => {
                    self.next_autonat_client_tick = OptionFuture::from(Some(
                        tokio::time::sleep(self.autonat_client_tick_interval)
                            .map(|()| addr)
                            .boxed(),
                    ))
                    .fuse();
                }
                Command::MapAddress(addr) => {
                    if let Err(e) = self.address_mapper_behaviour.try_map_address(addr) {
                        error!("Failed to start address mapping: {e}");
                    }
                }
                Command::NewExternalAddrCandidate(addr) => {
                    return Poll::Ready(ToSwarm::NewExternalAddrCandidate(addr));
                }
            }
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, pin::Pin};

    use libp2p::{
        autonat::v2::server,
        core::{
            transport::{DialOpts, ListenerId, MemoryTransport, Transport, TransportError},
            upgrade::Version,
        },
        identify,
        identity::Keypair,
        plaintext,
        swarm::{NetworkBehaviour, SwarmEvent},
        yamux, Swarm,
    };
    use libp2p_swarm_test::SwarmExt as _;
    use rand::{
        rngs::{OsRng, StdRng},
        SeedableRng as _,
    };

    use super::*;
    use crate::{
        behaviour::nat::address_mapper::errors::AddressMapperError, config::NatMappingSettings,
    };

    const TEST_PROTOCOL_VERSION: &str = "/test/1.0.0";

    thread_local! {
        static EXPECTED_ADDRESS: RefCell<Option<Multiaddr>> = const { RefCell::new(None) };
    }

    #[tokio::test]
    async fn nat_mapping_happy_flow() {
        let (server, server_addr) = create_server_with_blocking_transport();

        let expected_addr: Multiaddr = format!("/memory/{}", rand::random::<u32>())
            .parse()
            .unwrap();
        EXPECTED_ADDRESS.with(|a| *a.borrow_mut() = Some(expected_addr.clone()));

        let mut client = create_client::<TestMapper>(false);
        client.listen_on(expected_addr.clone()).unwrap();

        client.dial(server_addr).unwrap();

        let actual_events = collect_nat_events(client, server, expected_addr)
            .await
            .expect("Happy flow should complete");

        let expected_events = vec![
            "NewExternalAddrCandidate(private)",
            "AutoNATFailure",
            "NewExternalMappedAddress",
            "NewExternalAddrCandidate(mapped)",
            "NewExternalAddrCandidate(mapped)",
            "ExternalAddrConfirmed",
        ];

        assert_eq!(actual_events, expected_events);
    }

    #[tokio::test]
    async fn nat_mapping_failure_flow() {
        let (server, server_addr) = create_server_with_blocking_transport();

        let expected_addr: Multiaddr = format!("/memory/{}", rand::random::<u32>())
            .parse()
            .unwrap();

        EXPECTED_ADDRESS.with(|a| *a.borrow_mut() = None);

        let mut client = create_client::<TestMapper>(false);
        client.listen_on(expected_addr.clone()).unwrap();

        client.dial(server_addr).unwrap();

        let actual_events = collect_nat_events(client, server, expected_addr)
            .await
            .expect("Failure flow should complete");

        let expected_events = vec![
            "NewExternalAddrCandidate(private)",
            "AutoNATFailure",
            "MappingFailed",
        ];

        assert_eq!(actual_events, expected_events);
    }

    struct TestMapper;

    #[async_trait::async_trait]
    impl NatMapper for TestMapper {
        async fn map_address(
            _address_to_map: &Multiaddr,
            _settings: NatMappingSettings,
        ) -> Result<Multiaddr, AddressMapperError> {
            EXPECTED_ADDRESS.with(|a| {
                a.borrow().clone().map_or_else(
                    || {
                        Err(AddressMapperError::PortMappingFailed(
                            "All protocols failed".to_owned(),
                        ))
                    },
                    Ok,
                )
            })
        }
    }

    /// Transport that blocks the first dial attempt, then allows subsequent
    /// dials. This simulates a network scenario where the initial `AutoNAT`
    /// test fails (e.g., private address unreachable), triggering NAT
    /// mapping, and then subsequent connections succeed (e.g., to test the
    /// mapped address).
    #[derive(Default)]
    struct FirstDialBlockingTransport {
        inner: MemoryTransport,
        has_blocked_once: bool,
    }

    impl Transport for FirstDialBlockingTransport {
        type Output = <MemoryTransport as Transport>::Output;
        type Error = <MemoryTransport as Transport>::Error;
        type ListenerUpgrade = <MemoryTransport as Transport>::ListenerUpgrade;
        type Dial = <MemoryTransport as Transport>::Dial;

        fn listen_on(
            &mut self,
            id: ListenerId,
            addr: Multiaddr,
        ) -> Result<(), TransportError<Self::Error>> {
            self.inner.listen_on(id, addr)
        }

        fn remove_listener(&mut self, id: ListenerId) -> bool {
            self.inner.remove_listener(id)
        }

        fn dial(
            &mut self,
            addr: Multiaddr,
            opts: DialOpts,
        ) -> Result<Self::Dial, TransportError<Self::Error>> {
            if !self.has_blocked_once {
                self.has_blocked_once = true;
                return Err(TransportError::MultiaddrNotSupported(addr));
            }

            self.inner.dial(addr, opts)
        }

        fn poll(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<libp2p::core::transport::TransportEvent<Self::ListenerUpgrade, Self::Error>>
        {
            Pin::new(&mut self.inner).poll(cx)
        }
    }

    #[derive(NetworkBehaviour)]
    struct Server {
        autonat: server::Behaviour<OsRng>,
        identify: identify::Behaviour,
    }

    #[derive(NetworkBehaviour)]
    struct Client<Mapper: NatMapper + 'static> {
        nat: InnerNatBehaviour<StdRng, Mapper, SystemGatewayDetector>,
        identify: identify::Behaviour,
    }

    fn create_memory_swarm<Behaviour, BehaviourFactory>(
        use_blocking: bool,
        behaviour_fn: BehaviourFactory,
    ) -> Swarm<Behaviour>
    where
        Behaviour: NetworkBehaviour,
        BehaviourFactory: FnOnce(&Keypair) -> Behaviour,
    {
        let keypair = Keypair::generate_ed25519();
        let transport = if use_blocking {
            FirstDialBlockingTransport::default()
                .upgrade(Version::V1)
                .authenticate(plaintext::Config::new(&keypair))
                .multiplex(yamux::Config::default())
                .boxed()
        } else {
            MemoryTransport::default()
                .upgrade(Version::V1)
                .authenticate(plaintext::Config::new(&keypair))
                .multiplex(yamux::Config::default())
                .boxed()
        };

        libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_other_transport(|_| transport)
            .expect("transport should be supported")
            .with_behaviour(behaviour_fn)
            .expect("behaviour should be supported")
            .build()
    }

    fn create_server_with_blocking_transport() -> (Swarm<Server>, Multiaddr) {
        let mut swarm = create_memory_swarm(true, |keypair| Server {
            autonat: server::Behaviour::new(OsRng),
            identify: identify::Behaviour::new(identify::Config::new(
                TEST_PROTOCOL_VERSION.to_owned(),
                keypair.public(),
            )),
        });

        let server_addr: Multiaddr = format!("/memory/{}", rand::random::<u32>())
            .parse()
            .unwrap();

        swarm.listen_on(server_addr.clone()).unwrap();

        (swarm, server_addr)
    }

    fn create_client<Mapper: NatMapper + 'static>(use_blocking: bool) -> Swarm<Client<Mapper>> {
        let mut nat_settings = NatSettings::default();
        nat_settings.autonat.probe_interval_millisecs = Some(500);
        nat_settings.mapping.max_retries = 0;

        create_memory_swarm(use_blocking, move |keypair| {
            let nat = InnerNatBehaviour::create(
                StdRng::from_entropy(),
                nat_settings,
                AddressMapperBehaviour::<Mapper>::new(nat_settings.mapping),
                GatewayMonitor::<SystemGatewayDetector>::new(nat_settings.gateway_monitor),
            );

            let identify = identify::Behaviour::new(identify::Config::new(
                TEST_PROTOCOL_VERSION.to_owned(),
                keypair.public(),
            ));

            Client { nat, identify }
        })
    }

    async fn collect_nat_events<
        Mapper: NatMapper + 'static,
        ServerBehaviour: NetworkBehaviour + Send,
    >(
        mut client: Swarm<Client<Mapper>>,
        mut server: Swarm<ServerBehaviour>,
        expected_addr: Multiaddr,
    ) -> Result<Vec<String>, String>
    where
        ServerBehaviour::ToSwarm: std::fmt::Debug,
    {
        tokio::time::timeout(Duration::from_secs(30), async {
            let mut actual_events = Vec::new();

            loop {
                tokio::select! {
                    event = client.next_swarm_event() => {
                        match &event {
                            SwarmEvent::ExternalAddrConfirmed { address } => {
                                actual_events.push("ExternalAddrConfirmed".to_owned());
                                if *address == expected_addr { break; }
                            }
                            SwarmEvent::Behaviour(ClientEvent::Nat(Either::Right(mapping_event))) => {
                                match mapping_event {
                                    address_mapper::Event::NewExternalMappedAddress { .. } => {
                                        actual_events.push("NewExternalMappedAddress".to_owned());
                                    }
                                    address_mapper::Event::AddressMappingFailed(_) => {
                                        actual_events.push("MappingFailed".to_owned());
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            SwarmEvent::Behaviour(ClientEvent::Nat(Either::Left(autonat_event))) => {
                                actual_events.push(if autonat_event.result.is_err() { "AutoNATFailure" } else { "AutoNATSuccess" }.to_owned());
                            }
                            SwarmEvent::NewExternalAddrCandidate { address } => {
                                actual_events.push(if *address == expected_addr { "NewExternalAddrCandidate(mapped)" } else { "NewExternalAddrCandidate(private)" }.to_owned());
                            }
                            _ => {}
                        }
                    },
                    _ = server.next_swarm_event() => {}
                }
            }

            Ok::<Vec<String>, String>(actual_events)
        }).await.map_err(|_| "Test timeout".to_owned())?
    }
}
