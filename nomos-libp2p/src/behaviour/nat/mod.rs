use std::task::{Context, Poll};

use libp2p::{
    autonat::v2::client::{Behaviour, Config},
    core::{transport::PortUse, Endpoint},
    swarm::{
        behaviour::toggle::{Toggle, ToggleConnectionHandler},
        ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, NewListenAddr, THandler,
        THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};
use rand::RngCore;

enum State {
    TestIfPublic,
    TryPortMapping,
    TestIfMappedPublic,
    Public,
    MappedPublic,
    Private,
}

/// This behaviour is responsible for confirming that the addresses of the node
/// are publicly reachable.
pub struct NatBehaviour<R: RngCore + 'static> {
    /// True if the node is configured with a static public IP address.
    is_static_public: bool,
    /// The static public listen address is passed through this variable to
    /// the `poll()` method. Unused if the node is not configured with a static
    /// public IP address.
    static_listen_addr: Option<Multiaddr>,
    /// `AutoNAT` client behaviour which is used to confirm if addresses of our
    /// node are indeed publicly reachable. This behaviour is **disabled** if
    /// the node is configured with a static public IP address.
    autonat_client_behaviour: Toggle<Behaviour<R>>,
}

impl<R: RngCore + 'static> NatBehaviour<R> {
    pub fn new(rng: R, autonat_client_config: Option<Config>) -> Self {
        let is_static_public = autonat_client_config.is_none();

        let autonat_client_behaviour =
            Toggle::from(autonat_client_config.map(|config| Behaviour::new(rng, config)));

        Self {
            is_static_public,
            static_listen_addr: None,
            autonat_client_behaviour,
        }
    }
}

impl<R: RngCore + 'static> NetworkBehaviour for NatBehaviour<R> {
    type ConnectionHandler =
        ToggleConnectionHandler<<Behaviour as NetworkBehaviour>::ConnectionHandler>;

    type ToSwarm = <Behaviour as NetworkBehaviour>::ToSwarm;

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
        if self.is_static_public {
            match event {
                FromSwarm::NewListenAddr(NewListenAddr {
                    listener_id: _,
                    addr,
                }) => self.static_listen_addr = Some(addr.clone()),
                _ => {}
            }
        } else {
            self.autonat_client_behaviour.on_swarm_event(event);
        }
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
        if let Some(addr) = self.static_listen_addr.take() {
            return Poll::Ready(ToSwarm::ExternalAddrConfirmed(addr));
        }

        if let Poll::Ready(autonat_event) = self.autonat_client_behaviour.poll(cx) {
            return Poll::Ready(autonat_event);
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use libp2p::{
        autonat::{self},
        identify, identity,
        swarm::SwarmEvent,
        Swarm,
    };
    use libp2p_swarm_test::SwarmExt as _;
    use rand::rngs::OsRng;
    use tokio::time::timeout;
    use tracing_subscriber::{fmt::TestWriter, EnvFilter};

    use super::*;

    #[derive(NetworkBehaviour)]
    pub struct Client {
        nat: NatBehaviour<OsRng>,
        identify: identify::Behaviour,
    }

    impl Client {
        pub fn new(public_key: identity::PublicKey) -> Self {
            let nat = NatBehaviour::new(
                OsRng,
                Some(Config::default().with_probe_interval(Duration::from_millis(100))),
            );
            let identify =
                identify::Behaviour::new(identify::Config::new("/unittest".into(), public_key));
            Self { nat, identify }
        }
    }

    #[derive(NetworkBehaviour)]
    pub struct Server {
        autonat_server: autonat::v2::server::Behaviour<OsRng>,
        identify: identify::Behaviour,
    }

    impl Server {
        pub fn new(public_key: identity::PublicKey) -> Self {
            let autonat_server = autonat::v2::server::Behaviour::new(OsRng);
            let identify =
                identify::Behaviour::new(identify::Config::new("/unittest".into(), public_key));
            Self {
                autonat_server,
                identify,
            }
        }
    }

    #[tokio::test]
    async fn test_external_address_is_confirmed_by_autonat_server() {
        const _500MS: Duration = Duration::from_millis(500);
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .compact()
            .with_writer(TestWriter::default())
            .try_init();
        let mut client = Swarm::new_ephemeral_tokio(|keypair| Client::new(keypair.public()));
        let mut server = Swarm::new_ephemeral_tokio(|keypair| Server::new(keypair.public()));

        let (_, client_addr) = client.listen().await;
        let _ = server.listen().with_tcp_addr_external().await;

        let client_addr_clone = client_addr.clone();

        client.connect(&mut server).await;

        let client_task = timeout(_500MS, async move {
            // The address candidate is automatically taken by Identify from the address
            // observed by the server.
            let confirmed = client
                .wait(|e| match e {
                    SwarmEvent::ExternalAddrConfirmed { address } => Some(address),
                    _ => None,
                })
                .await;
            assert_eq!(confirmed, client_addr);
            let autonat::v2::client::Event {
                tested_addr,
                result,
                ..
            } = client
                .wait(|e| match e {
                    SwarmEvent::Behaviour(ClientEvent::Nat(event)) => Some(event),
                    _ => None,
                })
                .await;
            assert_eq!(tested_addr, client_addr);
            assert!(result.is_ok(), "Result: {result:?}");
        });

        let server_task = timeout(_500MS, async move {
            let autonat::v2::server::Event {
                tested_addr,
                result,
                ..
            } = server
                .wait(|e| match e {
                    SwarmEvent::Behaviour(ServerEvent::AutonatServer(event)) => Some(event),
                    _ => None,
                })
                .await;
            assert_eq!(tested_addr, client_addr_clone);
            assert!(result.is_ok(), "Result: {result:?}");
        });

        let (client_result, server_result) = tokio::join!(client_task, server_task);
        client_result.expect("No timeout");
        server_result.expect("No timeout");
    }
}
