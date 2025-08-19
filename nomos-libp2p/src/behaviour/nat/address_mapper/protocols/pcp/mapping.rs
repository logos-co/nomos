use std::{net::Ipv4Addr, num::NonZeroU16};

use zerocopy::FromBytes as _;

use crate::behaviour::nat::address_mapper::protocols::pcp::{
    client::PcpError,
    wire::{ipv6_mapped_to_ipv4, PcpMapResponse, Protocol, ResultCode, PCP_MAP_SIZE},
};

#[derive(Debug)]
pub struct Mapping {
    #[cfg_attr(not(test), expect(dead_code, reason = "Part of protocol"))]
    pub protocol: Protocol,
    pub local_ip: Ipv4Addr,
    pub local_port: NonZeroU16,
    #[cfg_attr(not(test), expect(dead_code, reason = "Part of protocol"))]
    pub gateway: Ipv4Addr,
    pub external_address: Ipv4Addr,
    pub external_port: NonZeroU16,
    pub lifetime_seconds: u32,
    #[cfg_attr(not(test), expect(dead_code, reason = "Part of protocol"))]
    pub nonce: [u8; 12],
    #[cfg_attr(not(test), expect(dead_code, reason = "Part of protocol"))]
    pub epoch_time: u32,
}

impl Mapping {
    pub fn from_map_response(
        data: &[u8],
        expected_protocol: Protocol,
        expected_port: NonZeroU16,
        expected_nonce: [u8; 12],
        local_ip: Ipv4Addr,
        gateway: Ipv4Addr,
    ) -> Result<Self, PcpError> {
        if data.len() != PCP_MAP_SIZE {
            return Err(PcpError::InvalidSize {
                expected: PCP_MAP_SIZE,
                actual: data.len(),
            });
        }

        let response = PcpMapResponse::read_from_bytes(data)
            .map_err(|e| PcpError::ParseError(format!("Failed to parse MAP response: {e}")))?;

        if response.nonce != expected_nonce {
            return Err(PcpError::NonceMismatch);
        }

        if response.protocol != expected_protocol as u8 {
            return Err(PcpError::ProtocolMismatch);
        }

        if response.internal_port.get() != expected_port.get() {
            return Err(PcpError::PortMismatch);
        }

        let Ok(result_code) = ResultCode::try_from(response.result_code) else {
            tracing::warn!("Unknown PCP result code: {}", response.result_code);

            return Err(PcpError::InvalidResponse(format!(
                "Unknown result code: {}",
                response.result_code
            )));
        };

        if result_code != ResultCode::Success {
            return Err(PcpError::ServerError(result_code));
        }

        let external_v4 =
            ipv6_mapped_to_ipv4(&response.assigned_external_address).ok_or_else(|| {
                PcpError::InvalidResponse(
                    "External address is not IPv4-mapped IPv6 format".to_owned(),
                )
            })?;

        let assigned_external_port = NonZeroU16::new(response.assigned_external_port.get())
            .ok_or_else(|| {
                PcpError::InvalidResponse(format!(
                    "Invalid external port: {}",
                    response.assigned_external_port.get()
                ))
            })?;

        Ok(Self {
            protocol: expected_protocol,
            local_ip,
            local_port: expected_port,
            gateway,
            external_port: assigned_external_port,
            external_address: external_v4,
            lifetime_seconds: response.lifetime.get(),
            nonce: response.nonce,
            epoch_time: response.epoch_time.get(),
        })
    }
}

#[cfg(test)]
mod tests {
    use zerocopy::{FromZeros as _, IntoBytes as _};

    use super::*;
    use crate::behaviour::nat::address_mapper::protocols::pcp::wire::{
        ipv4_to_ipv6_mapped, OPCODE_MAP, PCP_VERSION,
    };

    fn create_valid_map_response(
        nonce: [u8; 12],
        protocol: u8,
        internal_port: u16,
        external_port: u16,
        external_ip: Ipv4Addr,
        lifetime: u32,
        result_code: u8,
    ) -> Vec<u8> {
        let mut response = PcpMapResponse::new_zeroed();
        response.version = PCP_VERSION;
        response.opcode = OPCODE_MAP | 0x80;
        response.result_code = result_code;
        response.lifetime.set(lifetime);
        response.epoch_time.set(1_234_567_890);
        response.nonce = nonce;
        response.protocol = protocol;
        response.internal_port.set(internal_port);
        response.assigned_external_port.set(external_port);
        response.assigned_external_address = ipv4_to_ipv6_mapped(external_ip);
        response.as_bytes().to_vec()
    }

    #[test]
    fn test_mapping_from_valid_response() {
        let nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let local_ip = Ipv4Addr::new(192, 168, 1, 100);
        let gateway = Ipv4Addr::new(192, 168, 1, 1);
        let external_ip = Ipv4Addr::new(203, 0, 113, 50);
        let internal_port = NonZeroU16::new(8080).unwrap();
        let external_port = 8081;

        let response_data = create_valid_map_response(
            nonce,
            Protocol::Tcp as u8,
            internal_port.get(),
            external_port,
            external_ip,
            7200,
            ResultCode::Success as u8,
        );

        let mapping = Mapping::from_map_response(
            &response_data,
            Protocol::Tcp,
            internal_port,
            nonce,
            local_ip,
            gateway,
        )
        .unwrap();

        assert_eq!(mapping.protocol, Protocol::Tcp);
        assert_eq!(mapping.local_ip, local_ip);
        assert_eq!(mapping.local_port, internal_port);
        assert_eq!(mapping.gateway, gateway);
        assert_eq!(mapping.external_port.get(), external_port);
        assert_eq!(mapping.external_address, external_ip);
        assert_eq!(mapping.lifetime_seconds, 7200);
        assert_eq!(mapping.nonce, nonce);
        assert_eq!(mapping.epoch_time, 1_234_567_890);
    }

    #[test]
    fn test_mapping_invalid_size() {
        let short_data = vec![0u8; 10];
        let result = Mapping::from_map_response(
            &short_data,
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            [0; 12],
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );

        assert!(matches!(
            result,
            Err(PcpError::InvalidSize {
                expected: PCP_MAP_SIZE,
                actual: 10
            })
        ));

        let long_data = vec![0u8; PCP_MAP_SIZE + 10];
        let result = Mapping::from_map_response(
            &long_data,
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            [0; 12],
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );

        assert!(matches!(
            result,
            Err(PcpError::InvalidSize {
                expected: PCP_MAP_SIZE,
                actual: 70
            })
        ));
    }

    #[test]
    fn test_mapping_validation_errors() {
        let nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let wrong_nonce = [12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1];

        let response_data = create_valid_map_response(
            wrong_nonce,
            Protocol::Tcp as u8,
            8080,
            8081,
            Ipv4Addr::new(203, 0, 113, 50),
            7200,
            ResultCode::Success as u8,
        );
        let result = Mapping::from_map_response(
            &response_data,
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            nonce,
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );
        assert!(matches!(result, Err(PcpError::NonceMismatch)));

        let response_data = create_valid_map_response(
            nonce,
            Protocol::Udp as u8,
            8080,
            8081,
            Ipv4Addr::new(203, 0, 113, 50),
            7200,
            ResultCode::Success as u8,
        );
        let result = Mapping::from_map_response(
            &response_data,
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            nonce,
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );
        assert!(matches!(result, Err(PcpError::ProtocolMismatch)));

        let response_data = create_valid_map_response(
            nonce,
            Protocol::Tcp as u8,
            9000,
            8081,
            Ipv4Addr::new(203, 0, 113, 50),
            7200,
            ResultCode::Success as u8,
        );
        let result = Mapping::from_map_response(
            &response_data,
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            nonce,
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );
        assert!(matches!(result, Err(PcpError::PortMismatch)));

        let mut response = PcpMapResponse::new_zeroed();
        response.version = PCP_VERSION;
        response.opcode = OPCODE_MAP | 0x80;
        response.result_code = ResultCode::Success as u8;
        response.lifetime.set(7200);
        response.nonce = nonce;
        response.protocol = Protocol::Tcp as u8;
        response.internal_port.set(8080);
        response.assigned_external_port.set(8081);
        response.assigned_external_address =
            [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

        let result = Mapping::from_map_response(
            response.as_bytes(),
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            nonce,
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );
        assert!(
            matches!(result, Err(PcpError::InvalidResponse(msg)) if msg.contains("External address is not IPv4-mapped"))
        );

        let response_data = create_valid_map_response(
            nonce,
            Protocol::Tcp as u8,
            8080,
            0,
            Ipv4Addr::new(203, 0, 113, 50),
            7200,
            ResultCode::Success as u8,
        );
        let result = Mapping::from_map_response(
            &response_data,
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            nonce,
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );
        assert!(
            matches!(result, Err(PcpError::InvalidResponse(msg)) if msg.contains("Invalid external port: 0"))
        );
    }

    #[test]
    fn test_mapping_server_error() {
        let nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

        let error_codes = [
            ResultCode::NetworkFailure,
            ResultCode::NoResources,
            ResultCode::UnsupportedProtocol,
        ];

        for error_code in error_codes {
            let response_data = create_valid_map_response(
                nonce,
                Protocol::Tcp as u8,
                8080,
                8081,
                Ipv4Addr::new(203, 0, 113, 50),
                7200,
                error_code as u8,
            );

            let result = Mapping::from_map_response(
                &response_data,
                Protocol::Tcp,
                NonZeroU16::new(8080).unwrap(),
                nonce,
                Ipv4Addr::new(192, 168, 1, 100),
                Ipv4Addr::new(192, 168, 1, 1),
            );

            assert!(matches!(
                result,
                Err(PcpError::ServerError(err)) if err == error_code
            ));
        }
    }

    #[test]
    fn test_mapping_unknown_result_code() {
        let nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

        let response_data = create_valid_map_response(
            nonce,
            Protocol::Tcp as u8,
            8080,
            8081,
            Ipv4Addr::new(203, 0, 113, 50),
            7200,
            99,
        );

        let result = Mapping::from_map_response(
            &response_data,
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            nonce,
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );

        assert!(matches!(result, Err(PcpError::InvalidResponse(_))));
    }

    #[test]
    fn test_mapping_malformed_response_data() {
        let malformed_data = vec![0xFF; PCP_MAP_SIZE];

        let result = Mapping::from_map_response(
            &malformed_data,
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            [0; 12],
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_mapping_security_validations() {
        let nonce1 = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let nonce2 = [12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1];

        let response = create_valid_map_response(
            nonce2,
            Protocol::Tcp as u8,
            8080,
            8081,
            Ipv4Addr::new(203, 0, 113, 50),
            7200,
            ResultCode::Success as u8,
        );

        let result = Mapping::from_map_response(
            &response,
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            nonce1,
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );
        assert!(matches!(result, Err(PcpError::NonceMismatch)));

        let response = create_valid_map_response(
            nonce1,
            Protocol::Tcp as u8,
            9000,
            8081,
            Ipv4Addr::new(203, 0, 113, 50),
            7200,
            ResultCode::Success as u8,
        );

        let result = Mapping::from_map_response(
            &response,
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            nonce1,
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );
        assert!(matches!(result, Err(PcpError::PortMismatch)));

        let response = create_valid_map_response(
            nonce1,
            Protocol::Udp as u8,
            8080,
            8081,
            Ipv4Addr::new(203, 0, 113, 50),
            7200,
            ResultCode::Success as u8,
        );

        let result = Mapping::from_map_response(
            &response,
            Protocol::Tcp,
            NonZeroU16::new(8080).unwrap(),
            nonce1,
            Ipv4Addr::new(192, 168, 1, 100),
            Ipv4Addr::new(192, 168, 1, 1),
        );

        assert!(matches!(result, Err(PcpError::ProtocolMismatch)));
    }
}
