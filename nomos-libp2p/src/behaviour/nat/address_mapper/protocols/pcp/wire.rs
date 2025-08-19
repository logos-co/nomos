//! PCP wire format structures and constants.
//!
//! Zero-copy packet structures using `zerocopy` crate for parsing.
//! All multibyte fields use network byte order via `U16NE`/`U32NE` types.

use std::net::{Ipv4Addr, Ipv6Addr};

use num_enum::{IntoPrimitive, TryFromPrimitive};
use zerocopy::{
    byteorder::network_endian::{U16 as U16NE, U32 as U32NE},
    FromBytes, FromZeros as _, Immutable, IntoBytes, KnownLayout, Unaligned,
};

use crate::behaviour::nat::address_mapper::protocols::pcp::client::PcpError;

pub const PCP_VERSION: u8 = 2;
pub const PCP_ANNOUNCE_SIZE: usize = 24;
pub const PCP_MAP_SIZE: usize = 60;
pub const OPCODE_ANNOUNCE: u8 = 0;
pub const OPCODE_MAP: u8 = 1;
pub const OPCODE_RESPONSE_MASK: u8 = 0x7F;
pub const IPV6_SIZE: usize = 16;

/// Transport protocol for port mappings (IANA protocol numbers)
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive, IntoPrimitive)]
#[repr(u8)]
pub enum Protocol {
    /// TCP protocol (IANA 6)
    Tcp = 6,
    /// UDP protocol (IANA 17)
    Udp = 17,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive, IntoPrimitive)]
#[repr(u8)]
pub enum ResultCode {
    Success = 0,
    UnsupportedVersion = 1,
    NotAuthorized = 2,
    MalformedRequest = 3,
    UnsupportedOpcode = 4,
    UnsupportedOption = 5,
    MalformedOption = 6,
    NetworkFailure = 7,
    NoResources = 8,
    UnsupportedProtocol = 9,
    UserExceededQuota = 10,
    CannotProvideExternalAddress = 11,
    AddressMismatch = 12,
    ExcessiveRemotePeers = 13,
}

/// ANNOUNCE request packet (24 bytes total)
///
/// Used for server discovery and capability negotiation.
/// Lifetime field must be 0 for requests.
#[derive(Debug, FromBytes, IntoBytes, Unaligned, Immutable)]
#[repr(C)]
pub struct PcpAnnounceRequest {
    /// PCP protocol version (must be 2)
    pub version: u8,
    /// Operation code (0 for ANNOUNCE)
    pub opcode: u8,
    /// Reserved field (must be 0)
    pub reserved: [u8; 2],
    /// Requested lifetime in seconds (must be 0 for ANNOUNCE)
    pub lifetime: U32NE,
    /// Client IPv4 address as IPv4-mapped IPv6
    pub client_ip: [u8; 16],
}

/// ANNOUNCE response packet (24 bytes total)
///
/// Server responds with external IP discovery and epoch time.
/// Opcode has response bit set (0x80).
#[derive(Debug, FromBytes, IntoBytes, Unaligned, Immutable, KnownLayout)]
#[repr(C)]
pub struct PcpAnnounceResponse {
    /// PCP protocol version (always 2)
    pub version: u8,
    /// Operation code with response bit (0x80 for ANNOUNCE response)
    pub opcode: u8,
    /// Result code (0=success, see `ResultCode` enum)
    pub result_code: u8,
    /// Reserved field
    pub reserved1: u8,
    /// Server-assigned lifetime (usually 0 for ANNOUNCE)
    pub lifetime: U32NE,
    /// Server epoch time in seconds since boot
    pub epoch_time: U32NE,
    /// Reserved field (12 bytes)
    pub reserved2: [u8; 12],
}

/// MAP request packet (60 bytes total)
///
/// Creates, renews, or deletes port mappings.
/// Lifetime=0 deletes existing mapping.
#[derive(Debug, FromBytes, IntoBytes, Unaligned, Immutable)]
#[repr(C)]
pub struct PcpMapRequest {
    /// PCP protocol version (must be 2)
    pub version: u8,
    /// Operation code (1 for MAP)
    pub opcode: u8,
    /// Reserved field (must be 0)
    pub reserved1: [u8; 2],
    /// Requested lifetime in seconds (0=delete)
    pub lifetime: U32NE,
    /// Client IPv4 address as IPv4-mapped IPv6
    pub client_ip: [u8; 16],
    /// Random 96-bit nonce for request/response matching
    pub nonce: [u8; 12],
    /// Transport protocol (6=TCP, 17=UDP)
    pub protocol: u8,
    /// Reserved field (must be 0)
    pub reserved2: [u8; 3],
    /// Internal (private) port number
    pub internal_port: U16NE,
    /// Suggested external port (0=any)
    pub external_port: U16NE,
    /// Suggested external IPv4 as IPv4-mapped IPv6 (0=any)
    pub external_address: [u8; 16],
}

/// MAP response packet (60 bytes total)
///
/// Server response with assigned external endpoint and actual lifetime.
/// Nonce must match request for security validation.
#[derive(Debug, FromBytes, IntoBytes, Unaligned, Immutable, KnownLayout)]
#[repr(C)]
pub struct PcpMapResponse {
    /// PCP protocol version (always 2)
    pub version: u8,
    /// Operation code with response bit (0x81 for MAP response)
    pub opcode: u8,
    /// Result code (0=success, see `ResultCode` enum)
    pub result_code: u8,
    /// Reserved field
    pub reserved1: u8,
    /// Actual assigned lifetime in seconds
    pub lifetime: U32NE,
    /// Server epoch time in seconds since boot
    pub epoch_time: U32NE,
    /// Reserved field (12 bytes)
    pub reserved2: [u8; 12],
    /// Nonce from request (must match for validation)
    pub nonce: [u8; 12],
    /// Transport protocol from request
    pub protocol: u8,
    /// Reserved field
    pub reserved3: [u8; 3],
    /// Internal port from request
    pub internal_port: U16NE,
    /// Server-assigned external port number
    pub assigned_external_port: U16NE,
    /// Server-assigned external IPv4 as IPv4-mapped IPv6
    pub assigned_external_address: [u8; 16],
}

impl PcpAnnounceRequest {
    pub fn new(client_addr: Ipv4Addr) -> Self {
        let mut request = Self::new_zeroed();
        request.version = PCP_VERSION;
        request.opcode = OPCODE_ANNOUNCE;
        request.lifetime.set(0);
        request.client_ip = ipv4_to_ipv6_mapped(client_addr);
        request
    }
}

impl PcpAnnounceResponse {
    pub fn validate(data: &[u8]) -> Result<(), PcpError> {
        if data.len() != PCP_ANNOUNCE_SIZE {
            return Err(PcpError::InvalidSize {
                expected: PCP_ANNOUNCE_SIZE,
                actual: data.len(),
            });
        }

        let response = Self::ref_from_bytes(data)
            .map_err(|e| PcpError::ParseError(format!("Failed to parse ANNOUNCE response: {e}")))?;

        if response.version != PCP_VERSION {
            return Err(PcpError::InvalidResponse(format!(
                "Version mismatch: expected {}, got {}",
                PCP_VERSION, response.version
            )));
        }

        if response.opcode & OPCODE_RESPONSE_MASK != OPCODE_ANNOUNCE {
            return Err(PcpError::InvalidResponse(format!(
                "Unexpected opcode in response: expected {OPCODE_ANNOUNCE}, got {}",
                response.opcode & OPCODE_RESPONSE_MASK
            )));
        }

        let Ok(result_code) = ResultCode::try_from(response.result_code) else {
            tracing::warn!(
                "Unknown PCP result code in ANNOUNCE response: {}",
                response.result_code
            );

            return Err(PcpError::InvalidResponse(format!(
                "Unknown result code: {}",
                response.result_code
            )));
        };

        if result_code != ResultCode::Success {
            return Err(PcpError::ServerError(result_code));
        }

        Ok(())
    }
}

impl PcpMapRequest {
    pub fn new(
        client_addr: Ipv4Addr,
        protocol: u8,
        internal_port: u16,
        preferred_external: Option<(Ipv4Addr, u16)>,
        lifetime_seconds: u32,
        nonce: [u8; 12],
    ) -> Self {
        let mut request = Self::new_zeroed();

        request.version = PCP_VERSION;
        request.opcode = OPCODE_MAP;
        request.lifetime.set(lifetime_seconds);
        request.client_ip = ipv4_to_ipv6_mapped(client_addr);
        request.nonce = nonce;
        request.protocol = protocol;
        request.internal_port.set(internal_port);

        if let Some((addr, port)) = preferred_external {
            request.external_port.set(port);
            request.external_address = ipv4_to_ipv6_mapped(addr);
        }

        request
    }

    #[cfg(test)]
    pub(crate) fn new_delete(
        client_addr: Ipv4Addr,
        protocol: u8,
        internal_port: u16,
        nonce: [u8; 12],
    ) -> Self {
        let mut request = Self::new_zeroed();

        request.version = PCP_VERSION;
        request.opcode = OPCODE_MAP;
        request.lifetime.set(0);
        request.client_ip = ipv4_to_ipv6_mapped(client_addr);
        request.nonce = nonce;
        request.protocol = protocol;
        request.internal_port.set(internal_port);

        request
    }
}

/// Converts IPv4 to IPv4-mapped IPv6 format (`::ffff:a.b.c.d`) for PCP wire
/// format.
pub const fn ipv4_to_ipv6_mapped(ipv4: Ipv4Addr) -> [u8; IPV6_SIZE] {
    let [a, b, c, d] = ipv4.octets();
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, a, b, c, d]
}

/// Extracts IPv4 from IPv4-mapped IPv6 format.
pub fn ipv6_mapped_to_ipv4(addr: &[u8; IPV6_SIZE]) -> Option<Ipv4Addr> {
    let ipv6 = Ipv6Addr::from(*addr);
    ipv6.to_ipv4_mapped()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcp_announce_request_zero_lifetime() {
        let client_ip = Ipv4Addr::new(192, 168, 1, 100);
        let request = PcpAnnounceRequest::new(client_ip);

        assert_eq!(request.lifetime.get(), 0);
    }

    #[test]
    fn test_pcp_announce_response_validation() {
        let mut response = PcpAnnounceResponse::new_zeroed();
        response.version = PCP_VERSION;
        response.opcode = OPCODE_ANNOUNCE | 0x80;
        response.result_code = ResultCode::Success as u8;
        response.lifetime.set(7200);
        response.epoch_time.set(1_234_567_890);

        let data = response.as_bytes();
        assert!(PcpAnnounceResponse::validate(data).is_ok());

        let short_data = &data[..10];
        assert!(matches!(
            PcpAnnounceResponse::validate(short_data),
            Err(PcpError::InvalidSize { .. })
        ));

        let mut bad_version = PcpAnnounceResponse::new_zeroed();
        bad_version.version = 1;
        bad_version.opcode = OPCODE_ANNOUNCE | 0x80;
        bad_version.result_code = ResultCode::Success as u8;
        assert!(matches!(
            PcpAnnounceResponse::validate(bad_version.as_bytes()),
            Err(PcpError::InvalidResponse(_))
        ));

        let mut bad_opcode = PcpAnnounceResponse::new_zeroed();
        bad_opcode.version = PCP_VERSION;
        bad_opcode.opcode = OPCODE_MAP | 0x80;
        bad_opcode.result_code = ResultCode::Success as u8;
        assert!(matches!(
            PcpAnnounceResponse::validate(bad_opcode.as_bytes()),
            Err(PcpError::InvalidResponse(_))
        ));

        let mut server_error = PcpAnnounceResponse::new_zeroed();
        server_error.version = PCP_VERSION;
        server_error.opcode = OPCODE_ANNOUNCE | 0x80;
        server_error.result_code = ResultCode::NetworkFailure as u8;
        assert!(matches!(
            PcpAnnounceResponse::validate(server_error.as_bytes()),
            Err(PcpError::ServerError(ResultCode::NetworkFailure))
        ));
    }

    #[test]
    fn test_pcp_map_request_creation() {
        let client_ip = Ipv4Addr::new(192, 168, 1, 100);
        let nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let preferred_external = Some((Ipv4Addr::new(203, 0, 113, 1), 8080));

        let request = PcpMapRequest::new(
            client_ip,
            Protocol::Tcp as u8,
            8080,
            preferred_external,
            7200,
            nonce,
        );

        assert_eq!(request.version, PCP_VERSION);
        assert_eq!(request.opcode, OPCODE_MAP);
        assert_eq!(request.lifetime.get(), 7200);
        assert_eq!(request.client_ip, ipv4_to_ipv6_mapped(client_ip));
        assert_eq!(request.nonce, nonce);
        assert_eq!(request.protocol, Protocol::Tcp as u8);
        assert_eq!(request.internal_port.get(), 8080);
        assert_eq!(request.external_port.get(), 8080);
        assert_eq!(
            request.external_address,
            ipv4_to_ipv6_mapped(Ipv4Addr::new(203, 0, 113, 1))
        );
    }

    #[test]
    fn test_pcp_map_request_no_preferred() {
        let client_ip = Ipv4Addr::new(192, 168, 1, 100);
        let nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

        let request = PcpMapRequest::new(client_ip, Protocol::Udp as u8, 9000, None, 3600, nonce);

        assert_eq!(request.version, PCP_VERSION);
        assert_eq!(request.opcode, OPCODE_MAP);
        assert_eq!(request.lifetime.get(), 3600);
        assert_eq!(request.protocol, Protocol::Udp as u8);
        assert_eq!(request.internal_port.get(), 9000);
        assert_eq!(request.external_port.get(), 0);
        assert_eq!(request.external_address, [0; 16]);
    }

    #[test]
    fn test_pcp_map_request_delete() {
        let client_ip = Ipv4Addr::new(192, 168, 1, 100);
        let nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

        let request = PcpMapRequest::new_delete(client_ip, Protocol::Tcp as u8, 8080, nonce);

        assert_eq!(request.version, PCP_VERSION);
        assert_eq!(request.opcode, OPCODE_MAP);
        assert_eq!(request.lifetime.get(), 0);
        assert_eq!(request.client_ip, ipv4_to_ipv6_mapped(client_ip));
        assert_eq!(request.nonce, nonce);
        assert_eq!(request.protocol, Protocol::Tcp as u8);
        assert_eq!(request.internal_port.get(), 8080);
    }
}
