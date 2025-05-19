use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use libp2p::{
    bytes::BytesMut,
    core::{
        transport::{MemoryTransport, PortUse},
        upgrade::Version,
        Endpoint, Transport as _,
    },
    identity::Keypair,
    multiaddr::Protocol,
    swarm::{
        ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandlerInEvent,
        THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};
use log::info;
use nomos_da_messages::replication::ReplicationRequest;
use subnetworks_assignations::MembershipHandler;
use tokio::{io, net::UdpSocket};
use tracing::error;

use crate::{protocols::replication::behaviour::ReplicationBehaviour, SubnetworkId};

#[derive(Clone)]
pub struct AllNeighbours {
    neighbours: Arc<Mutex<HashSet<PeerId>>>,
    addresses: Arc<Mutex<HashMap<PeerId, libp2p::Multiaddr>>>,
}

impl Default for AllNeighbours {
    fn default() -> Self {
        Self::new()
    }
}

impl AllNeighbours {
    #[must_use]
    pub fn new() -> Self {
        Self {
            neighbours: Arc::new(Mutex::new(HashSet::new())),
            addresses: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn add_neighbour(&self, id: PeerId) {
        self.neighbours.lock().unwrap().insert(id);
    }

    pub fn update_addresses(&self, addressbook: Vec<(PeerId, libp2p::Multiaddr)>) {
        self.addresses.lock().unwrap().extend(addressbook);
    }
}

impl MembershipHandler for AllNeighbours {
    type NetworkId = SubnetworkId;
    type Id = PeerId;

    fn membership(&self, _self_id: &Self::Id) -> HashSet<Self::NetworkId> {
        std::iter::once(0).collect()
    }

    fn is_allowed(&self, _id: &Self::Id) -> bool {
        true
    }

    fn members_of(&self, _network_id: &Self::NetworkId) -> HashSet<Self::Id> {
        self.neighbours.lock().unwrap().clone()
    }

    fn members(&self) -> HashSet<Self::Id> {
        self.neighbours.lock().unwrap().clone()
    }

    fn last_subnetwork_id(&self) -> Self::NetworkId {
        0
    }

    fn get_address(&self, peer_id: &PeerId) -> Option<libp2p::Multiaddr> {
        self.addresses.lock().unwrap().get(peer_id).cloned()
    }
}

pub fn new_swarm_in_memory<TBehavior>(
    key: &Keypair,
    behavior: TBehavior,
) -> libp2p::Swarm<TBehavior>
where
    TBehavior: NetworkBehaviour + Send,
{
    libp2p::SwarmBuilder::with_existing_identity(key.clone())
        .with_tokio()
        .with_other_transport(|_| {
            let transport = MemoryTransport::default()
                .upgrade(Version::V1)
                .authenticate(libp2p::plaintext::Config::new(key))
                .multiplex(libp2p::yamux::Config::default())
                .timeout(Duration::from_secs(20));

            Ok(transport)
        })
        .unwrap()
        .with_behaviour(|_| behavior)
        .unwrap()
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(20)))
        .build()
}

// A wrapper around ReplicationBehaviour that allows tampering with outbound
// messages.
pub struct TamperingReplicationBehaviour<M> {
    inner: ReplicationBehaviour<M>,
    tamper_hook: Option<Arc<dyn Fn(ReplicationRequest) -> ReplicationRequest + Send + Sync>>,
}

impl<M> TamperingReplicationBehaviour<M>
where
    M: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>,
{
    pub fn new(inner: ReplicationBehaviour<M>) -> Self {
        Self {
            inner,
            tamper_hook: None,
        }
    }

    pub fn set_tamper_hook<F>(&mut self, f: F)
    where
        F: Fn(ReplicationRequest) -> ReplicationRequest + Send + Sync + 'static,
    {
        self.tamper_hook = Some(Arc::new(f));
    }

    pub(crate) fn send_message(&mut self, message: &ReplicationRequest) {
        let mut msg = message.clone();
        if let Some(ref hook) = self.tamper_hook {
            msg = hook(msg);
        }
        self.inner.send_message(&msg);
    }
}

impl<M> NetworkBehaviour for TamperingReplicationBehaviour<M>
where
    M: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId> + 'static,
{
    type ConnectionHandler = <ReplicationBehaviour<M> as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = <ReplicationBehaviour<M> as NetworkBehaviour>::ToSwarm;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        self.inner.handle_established_inbound_connection(
            connection_id,
            peer_id,
            local_addr,
            remote_addr,
        )
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        addr: &Multiaddr,
        role_override: Endpoint,
        port_use: PortUse,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        self.inner.handle_established_outbound_connection(
            connection_id,
            peer_id,
            addr,
            role_override,
            port_use,
        )
    }

    fn on_swarm_event(&mut self, event: FromSwarm<'_>) {
        self.inner.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        self.inner
            .on_connection_handler_event(peer_id, connection_id, event);
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        self.inner.poll(cx)
    }
}

fn modify_packet(packet: &mut BytesMut) {
    if !packet.is_empty() {
        packet[1] ^= 0x80; // Flip 1 bit in the second byte to modify 1-RTT
                           // packet data
    }
}

// Heuristically determine if the packet is likely QUIC application data.
fn is_probable_application_data(packet: &[u8]) -> bool {
    if packet.is_empty() {
        return false;
    }
    let first = packet[0];
    // Detect post-Handshake (1-RTT) packet
    (first & 0x80) == 0
}

fn multiaddr_to_socket_addr(multiaddr: &Multiaddr) -> io::Result<SocketAddr> {
    let iter = multiaddr.iter();
    let mut ip = None;
    let mut port = None;

    for component in iter {
        match component {
            Protocol::Ip4(addr) => ip = Some(std::net::IpAddr::V4(addr)),
            Protocol::Ip6(addr) => ip = Some(std::net::IpAddr::V6(addr)),
            Protocol::Udp(p) => port = Some(p),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unsupported protocol",
                ))
            }
        }
    }

    match (ip, port) {
        (Some(ip), Some(port)) => Ok(SocketAddr::new(ip, port)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Multiaddr must include IP and UDP port",
        )),
    }
}

// Start the two-way UDP mutation proxy
pub async fn start_udp_mutation_proxy(
    proxy_addr: Multiaddr,
    target_addr: Multiaddr,
) -> io::Result<()> {
    let proxy_socket_addr = multiaddr_to_socket_addr(&proxy_addr)?;
    let target_socket_addr = multiaddr_to_socket_addr(&target_addr)?;

    info!("UDP proxy listening on {proxy_socket_addr}, forwarding to {target_socket_addr}");

    let socket = Arc::new(UdpSocket::bind(proxy_socket_addr).await?);
    let client_addr = Arc::new(tokio::sync::Mutex::new(None::<SocketAddr>));
    let mut buf = vec![0u8; 65535];

    loop {
        let (len, src_addr) = match socket.recv_from(&mut buf).await {
            Ok((len, addr)) => (len, addr),
            Err(e) => {
                error!("recv_from error: {e}");
                continue;
            }
        };

        let mut data = BytesMut::from(&buf[..len]);

        let dest_addr = if src_addr == target_socket_addr {
            // From target → send to client
            let value = *client_addr.lock().await;
            if let Some(addr) = value {
                addr
            } else {
                error!("No client address recorded yet");
                continue;
            }
        } else {
            // From client → save and send to target
            *client_addr.lock().await = Some(src_addr);
            target_socket_addr
        };

        if is_probable_application_data(&data) {
            modify_packet(&mut data);
        }

        if let Err(e) = socket.send_to(&data, dest_addr).await {
            error!("send_to error: {e}");
        }
    }
}
