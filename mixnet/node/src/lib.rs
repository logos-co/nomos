mod client_notifier;
pub mod config;

use std::{error::Error, net::SocketAddr, time::Duration};

use client_notifier::ClientNotifier;
pub use config::MixnetNodeConfig;
use mixnet_protocol::Body;
use mixnet_topology::MixnetNodeId;
use mixnet_util::ConnectionPool;
use nym_sphinx::{
    addressing::nodes::NymNodeRoutingAddress, Delay, DestinationAddressBytes, NodeAddressBytes,
    Payload, PrivateKey,
};
pub use sphinx_packet::crypto::PRIVATE_KEY_SIZE;
use sphinx_packet::{crypto::PUBLIC_KEY_SIZE, ProcessedPacket, SphinxPacket};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

// A mix node that routes packets in the Mixnet.
pub struct MixnetNode {
    config: MixnetNodeConfig,
    pool: ConnectionPool,
}

impl MixnetNode {
    pub fn new(config: MixnetNodeConfig) -> Self {
        let pool = ConnectionPool::new(config.connection_pool_size);
        Self { config, pool }
    }

    pub fn id(&self) -> MixnetNodeId {
        self.public_key()
    }

    pub fn public_key(&self) -> [u8; PUBLIC_KEY_SIZE] {
        self.config.public_key()
    }

    const CLIENT_NOTI_CHANNEL_SIZE: usize = 100;

    pub async fn run(self) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        tracing::info!("Public key: {:?}", self.public_key());

        // Spawn a ClientNotifier
        let (client_tx, client_rx) = mpsc::channel(Self::CLIENT_NOTI_CHANNEL_SIZE);
        tokio::spawn(async move {
            if let Err(e) = ClientNotifier::run(self.config.client_listen_address, client_rx).await
            {
                tracing::error!("failed to run client notifier: {e}");
            }
        });

        //TODO: Accepting ad-hoc TCP conns for now. Improve conn handling.
        //TODO: Add graceful shutdown
        let listener = TcpListener::bind(self.config.listen_address).await?;
        tracing::info!(
            "Listening mixnet node connections: {}",
            self.config.listen_address
        );

        loop {
            match listener.accept().await {
                Ok((socket, remote_addr)) => {
                    tracing::debug!("Accepted incoming connection from {remote_addr:?}");

                    let client_tx = client_tx.clone();
                    let private_key = self.config.private_key;
                    let pool = self.pool.clone();
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_connection(
                            socket,
                            self.config.max_retries,
                            self.config.retry_delay,
                            pool,
                            private_key,
                            client_tx,
                        )
                        .await
                        {
                            tracing::error!("failed to handle conn: {e}");
                        }
                    });
                }
                Err(e) => tracing::warn!("Failed to accept incoming connection: {e}"),
            }
        }
    }

    async fn handle_connection(
        mut socket: TcpStream,
        max_retries: usize,
        retry_delay: Duration,
        pool: ConnectionPool,
        private_key: [u8; PRIVATE_KEY_SIZE],
        client_tx: mpsc::Sender<Body>,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        loop {
            let body = Body::read(&mut socket).await?;

            let pool = pool.clone();
            let private_key = PrivateKey::from(private_key);
            let client_tx = client_tx.clone();

            tokio::spawn(async move {
                if let Err(e) = Self::handle_body(
                    max_retries,
                    retry_delay,
                    body,
                    &pool,
                    &private_key,
                    &client_tx,
                )
                .await
                {
                    tracing::error!("failed to handle body: {e}");
                }
            });
        }
    }

    // TODO: refactor this fn to make it receive less arguments
    #[allow(clippy::too_many_arguments)]
    async fn handle_body(
        max_retries: usize,
        retry_delay: Duration,
        body: Body,
        pool: &ConnectionPool,
        private_key: &PrivateKey,
        client_tx: &mpsc::Sender<Body>,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        match body {
            Body::SphinxPacket(packet) => {
                Self::handle_sphinx_packet(pool, max_retries, retry_delay, private_key, packet)
                    .await
            }
            Body::FinalPayload(payload) => {
                Self::forward_body_to_client_notifier(
                    private_key,
                    client_tx,
                    Body::FinalPayload(payload),
                )
                .await
            }
            _ => unreachable!(),
        }
    }

    async fn handle_sphinx_packet(
        pool: &ConnectionPool,
        max_retries: usize,
        retry_delay: Duration,
        private_key: &PrivateKey,
        packet: Box<SphinxPacket>,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        match packet.process(private_key)? {
            ProcessedPacket::ForwardHop(packet, next_node_addr, delay) => {
                Self::forward_packet_to_next_hop(
                    pool,
                    max_retries,
                    retry_delay,
                    packet,
                    next_node_addr,
                    delay,
                )
                .await
            }
            ProcessedPacket::FinalHop(destination_addr, _, payload) => {
                Self::forward_payload_to_destination(
                    pool,
                    max_retries,
                    retry_delay,
                    payload,
                    destination_addr,
                )
                .await
            }
        }
    }

    async fn forward_body_to_client_notifier(
        _private_key: &PrivateKey,
        client_tx: &mpsc::Sender<Body>,
        body: Body,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        // TODO: Decrypt the final payload using the private key, if it's encrypted

        // Do not wait when the channel is full or no receiver exists
        client_tx.try_send(body)?;
        Ok(())
    }

    async fn forward_packet_to_next_hop(
        pool: &ConnectionPool,
        max_retries: usize,
        retry_delay: Duration,
        packet: Box<SphinxPacket>,
        next_node_addr: NodeAddressBytes,
        delay: Delay,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        tracing::debug!("Delaying the packet for {delay:?}");
        tokio::time::sleep(delay.to_duration()).await;

        Self::forward(
            pool,
            max_retries,
            retry_delay,
            Body::new_sphinx(packet),
            NymNodeRoutingAddress::try_from(next_node_addr)?,
        )
        .await
    }

    async fn forward_payload_to_destination(
        pool: &ConnectionPool,
        max_retries: usize,
        retry_delay: Duration,
        payload: Payload,
        destination_addr: DestinationAddressBytes,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        tracing::debug!("Forwarding final payload to destination mixnode");

        Self::forward(
            pool,
            max_retries,
            retry_delay,
            Body::new_final_payload(payload),
            NymNodeRoutingAddress::try_from_bytes(&destination_addr.as_bytes())?,
        )
        .await
    }

    async fn forward(
        pool: &ConnectionPool,
        max_retries: usize,
        retry_delay: Duration,
        body: Body,
        to: NymNodeRoutingAddress,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        let addr = SocketAddr::try_from(to)?;
        let arc_socket = pool.get_or_init(&addr).await?;

        if let Err(e) = {
            let mut socket = arc_socket.lock().await;
            body.write(&mut *socket).await
        } {
            tracing::error!("Failed to forward packet to {addr} with error: {e}. Retrying...");
            return mixnet_protocol::retry_backoff(
                addr,
                max_retries,
                retry_delay,
                body,
                arc_socket,
            )
            .await;
        }
        Ok(())
    }
}
