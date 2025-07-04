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

use crate::{
    behaviour::nat::{
        address_mapper,
        address_mapper::AddressMapperBehaviour,
        state_machine::{Command, StateMachine},
    },
    AutonatClientSettings,
};

type Task = BoxFuture<'static, Multiaddr>;

/// Provides dynamic NAT-status detection, improvement (via address mapping on
/// the NAT-box), and periodic maintenance capabilities.
pub struct InnerNatBehaviour<R: RngCore + 'static> {
    /// `AutoNAT` client behaviour which is used to confirm if addresses of our
    /// node are indeed publicly reachable.
    autonat_client_behaviour: autonat::v2::client::Behaviour<R>,
    /// The address mapper behaviour is used to map the node's addresses at the
    /// default gateway using one of the protocols: `PCP`, `NAT-PMP`,
    /// `UPNP-IGD`. The current implementation is a placeholder and does not
    /// perform any actual address mapping, and always generates a failure
    /// event.
    address_mapper_behaviour: AddressMapperBehaviour,
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
}

impl<R: RngCore + 'static> InnerNatBehaviour<R> {
    pub fn new(rng: R, autonat_client_config: AutonatClientSettings) -> Self {
        let address_mapper_behaviour = AddressMapperBehaviour::default();
        let autonat_client_behaviour =
            autonat::v2::client::Behaviour::new(rng, autonat_client_config.to_libp2p_config());

        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();

        let state_machine = StateMachine::new(command_tx);

        let autonat_client_tick_interval = Duration::from_millis(
            autonat_client_config.retest_successful_external_addresses_interval_millisecs,
        );

        Self {
            autonat_client_behaviour,
            address_mapper_behaviour,
            state_machine,
            command_rx,
            next_autonat_client_tick: OptionFuture::default().fuse(),
            autonat_client_tick_interval,
        }
    }
}

impl<R: RngCore + 'static> NetworkBehaviour for InnerNatBehaviour<R> {
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
                    self.address_mapper_behaviour.try_map_address(addr);
                }
                Command::NewExternalAddrCandidate(addr) => {
                    return Poll::Ready(ToSwarm::NewExternalAddrCandidate(addr));
                }
            }
        }

        Poll::Pending
    }
}
