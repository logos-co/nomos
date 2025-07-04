use std::net::{IpAddr, SocketAddr};

use igd_next::{
    aio::{tokio::Tokio, Gateway},
    PortMappingProtocol, SearchOptions,
};
use libp2p::Multiaddr;
use multiaddr::Protocol;

use crate::behaviour::nat::address_mapper::{
    errors::AddressMapperError, protocol::MappingProtocol,
};

type AddressWithProtocol = (SocketAddr, PortMappingProtocol);

pub struct UpnpProtocol {
    gateway: Gateway<Tokio>,
    gateway_external_ip: IpAddr,
}

#[async_trait::async_trait]
impl MappingProtocol for UpnpProtocol {
    async fn initialize() -> Result<Box<Self>, AddressMapperError>
    where
        Self: Sized,
    {
        let gateway = igd_next::aio::tokio::search_gateway(SearchOptions::default()).await?;

        let gateway_external_ip = gateway
            .get_external_ip()
            .await
            .map_err(|e| AddressMapperError::ExternalIpFailed(e.to_string()))?;

        tracing::info!("UPnP gateway found: {gateway_external_ip}");

        Ok(Box::new(Self {
            gateway,
            gateway_external_ip,
        }))
    }

    async fn map_address(&mut self, address: &Multiaddr) -> Result<Multiaddr, AddressMapperError> {
        let (internal_address, protocol) = multiaddr_to_socketaddr(address)?;
        let mapped_port = internal_address.port();

        self.gateway
            .add_port(
                protocol,
                // Request the same port as the internal address
                mapped_port,
                internal_address,
                0,
                "libp2p UPnP mapping",
            )
            .await?;

        let port = mapped_port;
        let external_addr = format!("/ip4/{}/tcp/{port}", self.gateway_external_ip);

        tracing::info!("Successfully added UPnP mapping: {external_addr}");

        Ok(external_addr.parse()?)
    }
}

fn multiaddr_to_socketaddr(addr: &Multiaddr) -> Result<AddressWithProtocol, AddressMapperError> {
    let Some(ip) = addr.iter().find_map(|protocol| match protocol {
        Protocol::Ip4(addr) => Some(IpAddr::V4(addr)),
        Protocol::Ip6(addr) => Some(IpAddr::V6(addr)),
        _ => None,
    }) else {
        return Err(AddressMapperError::NoIpAddress);
    };

    let Some((port, protocol)) = addr.iter().find_map(|protocol| match protocol {
        Protocol::Tcp(port) => Some((port, PortMappingProtocol::TCP)),
        Protocol::Udp(port) => Some((port, PortMappingProtocol::UDP)),
        _ => None,
    }) else {
        return Err(AddressMapperError::MultiaddrParseError(
            "No TCP or UDP port found in multiaddr".to_owned(),
        ));
    };

    Ok((SocketAddr::new(ip, port), protocol))
}
