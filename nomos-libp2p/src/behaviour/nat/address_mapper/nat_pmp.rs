use std::net::Ipv4Addr;

use multiaddr::{Multiaddr, Protocol as MaProto};
use natpmp::{new_tokio_natpmp, NatpmpAsync, Protocol, Response};
use tokio::{net::UdpSocket, time::timeout};

use crate::{
    behaviour::nat::address_mapper::{errors::AddressMapperError, protocol::NatMapper},
    config::NatMappingSettings,
};

type PortNumber = u16;

pub struct NatPmp {
    settings: NatMappingSettings,
    nat_pmp: NatpmpAsync<UdpSocket>,
}

impl NatPmp {
    async fn send_map_request(
        &self,
        protocol: Protocol,
        port: PortNumber,
    ) -> Result<(), AddressMapperError> {
        self.nat_pmp
            .send_port_mapping_request(protocol, port, port, self.settings.lease_duration)
            .await
            .map_err(|e| AddressMapperError::PortMappingFailed(e.to_string()))
    }

    async fn recv_map_response_public_port(&self) -> Result<PortNumber, AddressMapperError> {
        match self
            .nat_pmp
            .read_response_or_retry()
            .await
            .map_err(|e| AddressMapperError::PortMappingFailed(e.to_string()))?
        {
            Response::UDP(r) | Response::TCP(r) => Ok(r.public_port()),
            Response::Gateway(_) => Err(AddressMapperError::PortMappingFailed(
                "Expected UDP/TCP mapping response; got Gateway response".to_owned(),
            )),
        }
    }

    async fn query_public_ip(&mut self) -> Result<Ipv4Addr, AddressMapperError> {
        self.nat_pmp
            .send_public_address_request()
            .await
            .map_err(|e| AddressMapperError::GatewayDiscoveryFailed(e.to_string()))?;

        match self
            .nat_pmp
            .read_response_or_retry()
            .await
            .map_err(|e| AddressMapperError::GatewayDiscoveryFailed(e.to_string()))?
        {
            Response::Gateway(pa) => Ok(*pa.public_address()),
            other => Err(AddressMapperError::GatewayDiscoveryFailed(format!(
                "Expected PublicAddress response; got {other:?}"
            ))),
        }
    }
}

#[async_trait::async_trait]
impl NatMapper for NatPmp {
    async fn initialize(settings: NatMappingSettings) -> Result<Box<Self>, AddressMapperError>
    where
        Self: Sized,
    {
        let nat_pmp = new_tokio_natpmp()
            .await
            .map_err(|e| AddressMapperError::PortMappingFailed(e.to_string()))?;

        Ok(Box::new(Self { settings, nat_pmp }))
    }

    async fn map_address(
        &mut self,
        internal_address: &Multiaddr,
    ) -> Result<Multiaddr, AddressMapperError> {
        let (port, protocol) = extract_port_and_protocol(internal_address)?;

        self.send_map_request(protocol, port).await?;

        let public_port = timeout(self.settings.timeout, self.recv_map_response_public_port())
            .await
            .map_err(|_| {
                AddressMapperError::PortMappingFailed(
                    "Timeout waiting for NAT-PMP mapping response".to_owned(),
                )
            })??;

        let public_ip = timeout(self.settings.timeout, self.query_public_ip())
            .await
            .map_err(|_| {
                AddressMapperError::PortMappingFailed(
                    "Timeout waiting for NAT-PMP public IP response".to_owned(),
                )
            })??;

        build_public_address(internal_address, public_ip, public_port)
    }
}

fn extract_port_and_protocol(
    addr: &Multiaddr,
) -> Result<(PortNumber, Protocol), AddressMapperError> {
    addr.iter()
        .find_map(|p| match p {
            MaProto::Tcp(p) => Some((p, Protocol::TCP)),
            MaProto::Udp(p) => Some((p, Protocol::UDP)),
            _ => None,
        })
        .ok_or_else(|| {
            AddressMapperError::MultiaddrParseError(
                "No TCP or UDP port found in multiaddr".to_owned(),
            )
        })
}

fn build_public_address(
    internal: &Multiaddr,
    public_ip: Ipv4Addr,
    public_port: PortNumber,
) -> Result<Multiaddr, AddressMapperError> {
    let with_ip = internal
        .replace(0, |_| Some(multiaddr::Protocol::Ip4(public_ip)))
        .ok_or_else(|| {
            AddressMapperError::MultiaddrParseError("No IP address found in multiaddr".to_owned())
        })?;

    with_ip
        .replace(1, |p| match p {
            multiaddr::Protocol::Tcp(_) => Some(multiaddr::Protocol::Tcp(public_port)),
            multiaddr::Protocol::Udp(_) => Some(multiaddr::Protocol::Udp(public_port)),
            _ => None,
        })
        .ok_or_else(|| {
            AddressMapperError::MultiaddrParseError(
                "No TCP or UDP port found in multiaddr".to_owned(),
            )
        })
}
