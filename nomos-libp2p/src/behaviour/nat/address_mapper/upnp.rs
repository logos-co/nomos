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

pub struct UpnpProtocol {
    gateway: Option<Gateway<Tokio>>,
    gateway_external_ip: Option<IpAddr>,
}

impl UpnpProtocol {
    pub const fn new() -> Self {
        Self {
            gateway: None,
            gateway_external_ip: None,
        }
    }
}

#[async_trait::async_trait]
impl MappingProtocol for UpnpProtocol {
    async fn initialize(&mut self) -> Result<(), AddressMapperError> {
        let gateway = igd_next::aio::tokio::search_gateway(SearchOptions::default()).await?;

        let gateway_external_ip = gateway
            .get_external_ip()
            .await
            .map_err(|e| AddressMapperError::ExternalIpFailed(e.to_string()))?;

        self.gateway = Some(gateway);
        self.gateway_external_ip = Some(gateway_external_ip);

        Ok(())
    }

    async fn map_address(&mut self, address: &Multiaddr) -> Result<Multiaddr, AddressMapperError> {
        let gateway = self
            .gateway
            .as_mut()
            .ok_or(AddressMapperError::GatewayNotInitialized)?;

        let external_ip = self
            .gateway_external_ip
            .ok_or(AddressMapperError::GatewayNotInitialized)?;

        let internal_address = multiaddr_to_socketaddr(address)?;
        let mapped_port = internal_address.port();

        gateway
            .add_port(
                PortMappingProtocol::TCP,
                // Request the same port as the internal address
                mapped_port,
                internal_address,
                0,
                "libp2p UPnP mapping",
            )
            .await?;

        let port = mapped_port;
        let external_addr = format!("/ip4/{external_ip}/tcp/{port}");

        tracing::info!("Successfully added UPnP mapping: {external_addr}");

        Ok(external_addr.parse()?)
    }
}

fn multiaddr_to_socketaddr(addr: &Multiaddr) -> Result<SocketAddr, AddressMapperError> {
    let ip = addr
        .iter()
        .find_map(|protocol| match protocol {
            Protocol::Ip4(addr) => Some(IpAddr::V4(addr)),
            Protocol::Ip6(addr) => Some(IpAddr::V6(addr)),
            _ => None,
        })
        .ok_or(AddressMapperError::NoIpAddress)?;

    let port = addr
        .iter()
        .find_map(|protocol| match protocol {
            Protocol::Tcp(p) => Some(p),
            _ => None,
        })
        .ok_or(AddressMapperError::NoTcpPort)?;

    Ok(SocketAddr::new(ip, port))
}
