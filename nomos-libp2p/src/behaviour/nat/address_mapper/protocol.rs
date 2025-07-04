use libp2p::Multiaddr;

use crate::behaviour::nat::address_mapper::{errors::AddressMapperError, upnp::UpnpProtocol};

/// Trait for NAT mapping protocols
#[async_trait::async_trait]
pub trait MappingProtocol: Send + Sync + 'static {
    /// Initialize the protocol
    async fn initialize(&mut self) -> Result<(), AddressMapperError>;

    /// Map a TCP address and return the external address
    async fn map_address(&mut self, address: &Multiaddr) -> Result<Multiaddr, AddressMapperError>;
}

pub struct ProtocolManager {
    upnp: Box<dyn MappingProtocol>,
}

impl ProtocolManager {
    pub fn new() -> Self {
        Self {
            // Currently only UPnP, later add support for PCP and NAT-PMP
            upnp: Box::new(UpnpProtocol::new()),
        }
    }

    pub async fn initialize(&mut self) -> Result<(), AddressMapperError> {
        self.upnp.initialize().await?;

        tracing::info!("Initialized UPnP protocol");
        Ok(())
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
