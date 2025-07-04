use libp2p::Multiaddr;

use crate::behaviour::nat::address_mapper::{errors::AddressMapperError, upnp::UpnpProtocol};

/// Trait for NAT mapping protocols
#[async_trait::async_trait]
pub trait MappingProtocol: Send + Sync + 'static {
    /// Initialize the protocol
    async fn initialize() -> Result<Box<Self>, AddressMapperError>
    where
        Self: Sized;

    /// Map a TCP address and return the external address
    async fn map_address(&mut self, address: &Multiaddr) -> Result<Multiaddr, AddressMapperError>;
}

pub struct ProtocolManager {
    upnp: Box<dyn MappingProtocol>,
}

impl ProtocolManager {
    pub async fn initialize() -> Result<Self, AddressMapperError> {
        let upnp = UpnpProtocol::initialize().await?;
        Ok(Self { upnp })
    }

    pub async fn try_map_address(
        &mut self,
        address: &Multiaddr,
    ) -> Result<Multiaddr, AddressMapperError> {
        let external_address = self.upnp.map_address(address).await?;

        tracing::info!(
            "Successfully mapped {} to {} using {}",
            address,
            external_address,
            "UPnP"
        );

        Ok(external_address)
    }
}
