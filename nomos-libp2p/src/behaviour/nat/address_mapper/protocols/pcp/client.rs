use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    num::NonZeroU16,
    time::Duration,
};

use thiserror::Error;
#[cfg(test)]
use zerocopy::IntoBytes as _;

use crate::behaviour::nat::address_mapper::protocols::pcp::{
    connection::{generate_nonce, PcpConnection},
    mapping::Mapping,
    wire::{PcpAnnounceRequest, PcpAnnounceResponse, PcpMapRequest, Protocol, ResultCode},
};

pub type IpPort = (Ipv4Addr, NonZeroU16);

#[derive(Debug, Error)]
pub enum PcpError {
    #[error("Network IO error: {0}")]
    Network(#[from] std::io::Error),
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    #[error("Failed to parse response: {0}")]
    ParseError(String),
    #[error("Server returned error: {0:?}")]
    ServerError(ResultCode),
    #[error("Response nonce does not match request")]
    NonceMismatch,
    #[error("Response protocol does not match request")]
    ProtocolMismatch,
    #[error("Response port does not match request")]
    PortMismatch,
    #[error("Request timed out: {0}")]
    Timeout(#[source] tokio::time::error::Elapsed),
    #[error("Invalid response size: expected {expected} bytes, got {actual} bytes")]
    InvalidSize { expected: usize, actual: usize },
    #[error("PCP requires IPv4 server address, got IPv6: {0}")]
    IPv6ServerNotSupported(SocketAddr),
    #[error("Cannot get default gateway")]
    CannotGetGateway,
}

pub const PCP_PORT: u16 = 5351;
pub const DEFAULT_MAPPING_LIFETIME: u32 = 7200; // 2 hours

#[derive(Debug, Clone)]
pub struct PcpConfig {
    /// Default mapping lifetime in seconds
    pub mapping_lifetime: u32,
    /// Initial retry delay in milliseconds
    pub initial_retry_ms: u64,
    /// Maximum retry delay in seconds
    pub max_retry_delay_secs: u64,
    /// Maximum number of retry attempts
    pub max_retries: usize,
    /// Request timeout duration
    pub request_timeout: Duration,
}

impl Default for PcpConfig {
    fn default() -> Self {
        Self {
            mapping_lifetime: DEFAULT_MAPPING_LIFETIME,
            initial_retry_ms: 250,
            max_retry_delay_secs: 1,
            max_retries: 5,
            request_timeout: Duration::from_secs(1),
        }
    }
}

/// PCP (RFC 6887) client for NAT port mapping.
///
/// Protocol flow:
/// 1. ANNOUNCE: Discovers PCP server and learns external IP (0-byte lifetime)
/// 2. MAP: Creates port mappings with nonce-based request/response matching
/// 3. DELETE: Removes mappings via MAP with 0-byte lifetime
///
/// All requests use UDP port 5351 with exponential backoff retry.
/// Nonce prevent replay attacks and ensure response authenticity.
#[derive(Debug, Clone)]
pub struct PcpClient {
    gateway: SocketAddr,
    client_addr: Ipv4Addr,
    config: PcpConfig,
}

impl PcpClient {
    /// Creates a new PCP client with the specified configuration.
    ///
    /// Automatically discovers the default gateway for PCP communication.
    pub fn new(client_addr: Ipv4Addr, config: PcpConfig) -> Result<Self, PcpError> {
        let gateway_ip = Self::get_default_gateway()?;
        let gateway = SocketAddr::new(IpAddr::V4(gateway_ip), PCP_PORT);
        Ok(Self {
            gateway,
            client_addr,
            config,
        })
    }

    /// Tests if a PCP server is available by sending an ANNOUNCE request.
    ///
    /// Returns `true` if the server responds successfully, `false` otherwise.
    pub async fn probe_available(&self) -> bool {
        self.announce().await.is_ok()
    }

    /// Sends an ANNOUNCE request to discover the PCP server.
    pub async fn announce(&self) -> Result<(), PcpError> {
        tracing::debug!("Sending PCP ANNOUNCE to {}", self.gateway);

        let connection =
            PcpConnection::connect(self.client_addr, self.gateway, self.config.clone()).await?;

        let request = PcpAnnounceRequest::new(self.client_addr);

        let response = connection.send_announce_request(&request).await?;

        match PcpAnnounceResponse::validate(&response) {
            Ok(()) => {
                tracing::debug!("PCP ANNOUNCE successful");
                Ok(())
            }
            Err(e) => {
                tracing::warn!("PCP ANNOUNCE failed: {}", e);
                Err(e)
            }
        }
    }

    /// Requests a port mapping from the PCP server.
    ///
    /// Creates a mapping for the specified protocol and internal port.
    /// Optionally accepts a preferred external IP and port.
    ///
    /// Returns the server-assigned external endpoint and mapping details.
    pub async fn request_mapping(
        &self,
        protocol: Protocol,
        internal_port: NonZeroU16,
        preferred_external: Option<IpPort>,
    ) -> Result<Mapping, PcpError> {
        let nonce = generate_nonce();

        let request = PcpMapRequest::new(
            self.client_addr,
            protocol as u8,
            internal_port.get(),
            preferred_external.map(|(addr, port)| (addr, port.get())),
            self.config.mapping_lifetime,
            nonce,
        );

        let connection =
            PcpConnection::connect(self.client_addr, self.gateway, self.config.clone()).await?;

        let response = connection.send_map_request(&request).await?;

        let gateway_ip = match self.gateway.ip() {
            IpAddr::V4(v4) => v4,
            IpAddr::V6(_) => {
                return Err(PcpError::IPv6ServerNotSupported(self.gateway));
            }
        };

        match Mapping::from_map_response(
            &response,
            protocol,
            internal_port,
            nonce,
            self.client_addr,
            gateway_ip,
        ) {
            Ok(mapping) => {
                Self::log_mapping_success(&mapping);
                Ok(mapping)
            }
            Err(e) => {
                tracing::warn!("PCP mapping failed: {}", e);
                Err(e)
            }
        }
    }

    fn log_mapping_success(mapping: &Mapping) {
        tracing::info!(
            "PCP mapping created: {}:{} -> {}:{} (lifetime: {}s)",
            mapping.local_ip,
            mapping.local_port,
            mapping.external_address,
            mapping.external_port,
            mapping.lifetime_seconds
        );
    }

    fn get_default_gateway() -> Result<Ipv4Addr, PcpError> {
        if let Ok(ipv4_addrs) = netdev::get_default_gateway().map(|g| g.ipv4) {
            if let Some(gw) = ipv4_addrs.first() {
                return Ok(*gw);
            }
        }
        Err(PcpError::CannotGetGateway)
    }

    /// Releases an existing port mapping by sending a DELETE request.
    ///
    /// Uses the mapping's original nonce to identify and delete the specific
    /// mapping.
    #[cfg(test)]
    pub async fn release_mapping(&self, mapping: &Mapping) -> Result<(), PcpError> {
        self.send_delete_request(mapping).await?;
        Self::log_release_success(mapping);
        Ok(())
    }

    #[cfg(test)]
    async fn send_delete_request(&self, mapping: &Mapping) -> Result<(), PcpError> {
        let connection =
            PcpConnection::connect(self.client_addr, self.gateway, self.config.clone()).await?;

        let request = PcpMapRequest::new_delete(
            mapping.local_ip,
            mapping.protocol as u8,
            mapping.local_port.get(),
            mapping.nonce,
        );

        let response = connection.send_with_retry(request.as_bytes()).await?;

        Self::validate_delete_response(&response, &request.nonce)
    }

    #[cfg(test)]
    fn validate_delete_response(
        response: &[u8],
        expected_nonce: &[u8; 12],
    ) -> Result<(), PcpError> {
        use zerocopy::FromBytes as _;

        use crate::behaviour::nat::address_mapper::protocols::pcp::wire::{
            PcpMapResponse, PCP_MAP_SIZE,
        };

        if response.len() != PCP_MAP_SIZE {
            return Err(PcpError::InvalidSize {
                expected: PCP_MAP_SIZE,
                actual: response.len(),
            });
        }

        let map_response = PcpMapResponse::read_from_bytes(response)
            .map_err(|e| PcpError::ParseError(format!("Failed to parse MAP response: {e}")))?;

        if &map_response.nonce != expected_nonce {
            return Err(PcpError::NonceMismatch);
        }

        let Ok(result_code) = ResultCode::try_from(map_response.result_code) else {
            tracing::warn!(
                "Unknown PCP result code in MAP delete response: {}",
                map_response.result_code
            );

            return Err(PcpError::InvalidResponse(format!(
                "Unknown result code: {}",
                map_response.result_code
            )));
        };

        if result_code != ResultCode::Success {
            return Err(PcpError::ServerError(result_code));
        }

        Ok(())
    }

    #[cfg(test)]
    fn log_release_success(mapping: &Mapping) {
        tracing::info!(
            "PCP mapping released: {}:{} -> {}:{}",
            mapping.local_ip,
            mapping.local_port,
            mapping.external_address,
            mapping.external_port
        );
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    /// Run with: `PCP_CLIENT_IP=192.168.1.100 cargo test
    /// test_request_port_mapping -- --ignored`
    #[tokio::test]
    #[ignore = "Integration test - requires PCP_CLIENT_IP env var"]
    async fn test_request_port_mapping() {
        let client_ip = std::env::var("PCP_CLIENT_IP")
            .expect("PCP_CLIENT_IP environment variable required")
            .parse()
            .expect("PCP_CLIENT_IP must be valid IPv4 address");

        let client =
            PcpClient::new(client_ip, PcpConfig::default()).expect("Failed to create PCP client");

        if client.probe_available().await {
            println!("PCP server is available");

            println!("Requesting port mapping...");
            match client
                .request_mapping(Protocol::Tcp, NonZeroU16::new(8080).unwrap(), None)
                .await
            {
                Ok(mapping) => {
                    println!("Got mapping: {mapping:?}");

                    assert_eq!(mapping.protocol, Protocol::Tcp);
                    assert_eq!(mapping.local_port.get(), 8080);
                    assert!(mapping.lifetime_seconds > 0);

                    match client.release_mapping(&mapping).await {
                        Ok(()) => println!("Mapping released successfully"),
                        Err(e) => println!("Failed to release: {e}"),
                    }
                }
                Err(e) => {
                    println!("Port mapping failed: {e}");
                }
            }
        } else {
            println!("PCP server not available");
        }
        panic!("Panic");
    }
}
