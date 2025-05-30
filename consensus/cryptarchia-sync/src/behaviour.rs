use std::task::{Context, Poll};

use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        behaviour::ConnectionEstablished, ConnectionClosed, ConnectionDenied, ConnectionId,
        FromSwarm, NetworkBehaviour, THandlerInEvent, ToSwarm,
    },
    Multiaddr, PeerId, StreamProtocol,
};
use libp2p_stream::{Behaviour as StreamBehaviour, Control, IncomingStreams};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::messages::{Block, HeaderId};

#[derive(Error, Debug, Clone)]
pub enum ChainSyncError {
    #[error("Failed to start chain sync: {0}")]
    StartSyncError(String),
}

/// Events emitted by the `Behaviour` to the swarm.
#[derive(Debug)]
pub enum Event {
    ProvideBlocksRequest {
        /// Return blocks up to `target_block` if specified.
        target_block: Option<HeaderId>,
        /// The latest block at the tip of the local chain.
        local_tip: HeaderId,
        /// The latest immutable block.
        latest_immutable_block: HeaderId,
        /// The list of additional blocks that the requester has.
        additional_blocks: Vec<HeaderId>,
        /// Channel to stream blocks back to the Behavior.
        blocks_stream: mpsc::Sender<Block>,
    },
}

const SYNC_PROTOCOL_ID: &str = "/nomos/cryptarchia/sync/1.0.0";

const SYNC_PROTOCOL: StreamProtocol = StreamProtocol::new(SYNC_PROTOCOL_ID);

#[expect(dead_code, reason = "WIP")]
pub struct Behaviour {
    /// The underlying stream behavior that handles the actual streaming of
    /// blocks.
    stream_behaviour: StreamBehaviour,
    /// Control to open new streams.
    control: Control,
    /// A handle to inbound streams.
    incoming_streams: IncomingStreams,
    /// List of connected peers.
    peers: Vec<PeerId>,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self::new()
    }
}

impl Behaviour {
    #[must_use]
    pub fn new() -> Self {
        let stream_behaviour = StreamBehaviour::new();
        let mut control = stream_behaviour.new_control();
        let incoming_streams = control
            .accept(SYNC_PROTOCOL)
            .expect("Failed to accept incoming streams for sync protocol");
        Self {
            stream_behaviour,
            control,
            incoming_streams,
            peers: Vec::new(),
        }
    }

    pub fn start_sync(
        &mut self,
        _target_block: Option<HeaderId>,
        _local_tip: HeaderId,
        _immutable_block: HeaderId,
        _additional_blocks: Vec<HeaderId>,
        _rx_blocks: mpsc::Sender<Block>,
    ) -> Result<(), ChainSyncError> {
        Ok(())
    }
    fn add_peer(&mut self, peer: PeerId) {
        if !self.peers.contains(&peer) {
            self.peers.push(peer);
        }
    }

    fn remove_peer(&mut self, peer: &PeerId) {
        self.peers.retain(|p| p != peer);
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = <StreamBehaviour as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        self.stream_behaviour.handle_established_inbound_connection(
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
        port: PortUse,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        self.stream_behaviour
            .handle_established_outbound_connection(
                connection_id,
                peer_id,
                addr,
                role_override,
                port,
            )
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        match event {
            FromSwarm::ConnectionEstablished(ConnectionEstablished { peer_id, .. }) => {
                self.add_peer(peer_id);
            }
            FromSwarm::ConnectionClosed(ConnectionClosed { peer_id, .. }) => {
                self.remove_peer(&peer_id);
            }

            _ => {}
        }
        self.stream_behaviour.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        conn_id: ConnectionId,
        event: <Self::ConnectionHandler as libp2p::swarm::ConnectionHandler>::ToBehaviour,
    ) {
        self.stream_behaviour
            .on_connection_handler_event(peer_id, conn_id, event);
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        Poll::Pending
    }
}
