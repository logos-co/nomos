use std::net::{Ipv4Addr, SocketAddr};

use backon::{ExponentialBuilder, Retryable as _};
use rand::RngCore as _;
use tokio::{
    net::UdpSocket,
    time::{timeout, Duration},
};
use zerocopy::IntoBytes as _;

use crate::behaviour::nat::address_mapper::protocols::pcp::{
    client::{PcpConfig, PcpError},
    wire::{PcpAnnounceRequest, PcpMapRequest},
};

const BUFFER_SIZE: usize = 1024;
/// Let OS choose any available port for client socket
const ANY_PORT: u16 = 0;

/// PCP connection for reliable communication with retry logic
///
/// Handles connection to PCP server with exponential backoff retry.
/// Each connection is bound to client IP and connected to server.
pub struct PcpConnection {
    socket: UdpSocket,
    config: PcpConfig,
}

impl PcpConnection {
    pub async fn connect(
        client_addr: Ipv4Addr,
        server_addr: SocketAddr,
        config: PcpConfig,
    ) -> Result<Self, PcpError> {
        let socket = UdpSocket::bind((client_addr, ANY_PORT)).await?;

        socket.connect(server_addr).await?;

        Ok(Self { socket, config })
    }

    pub async fn send_with_retry(&self, request: &[u8]) -> Result<Vec<u8>, PcpError> {
        let backoff = create_backoff(
            self.config.initial_retry_ms,
            self.config.max_retry_delay_secs,
            self.config.max_retries,
        );

        (|| async {
            self.socket.send(request).await?;

            let mut buffer = vec![0u8; BUFFER_SIZE];

            match timeout(self.config.request_timeout, self.socket.recv(&mut buffer)).await {
                Ok(Ok(size)) => {
                    buffer.truncate(size);
                    Ok(buffer)
                }
                Ok(Err(e)) => Err(PcpError::from(e)),
                Err(elapsed) => Err(PcpError::Timeout(elapsed)),
            }
        })
        .retry(&backoff)
        .await
    }

    pub async fn send_announce_request(
        &self,
        request: &PcpAnnounceRequest,
    ) -> Result<Vec<u8>, PcpError> {
        self.send_with_retry(request.as_bytes()).await
    }

    pub async fn send_map_request(&self, request: &PcpMapRequest) -> Result<Vec<u8>, PcpError> {
        self.send_with_retry(request.as_bytes()).await
    }
}

/// Implements exponential backoff as specified in RFC 6887 Section 8.1.1:
/// "Clients should use exponential backoff with jitter to avoid thundering
/// herd"
fn create_backoff(
    initial_retry_ms: u64,
    max_delay_secs: u64,
    max_retries: usize,
) -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(initial_retry_ms))
        .with_max_delay(Duration::from_secs(max_delay_secs))
        .with_max_times(max_retries)
        .with_jitter()
}

/// Generate a random 12-byte nonce for PCP requests
pub fn generate_nonce() -> [u8; 12] {
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    nonce
}
