use libp2p::Multiaddr;

use crate::{
    behaviour::nat::address_mapper::{
        errors::AddressMapperError, nat_pmp::NatPmp, upnp::UpnpProtocol,
    },
    config::NatMappingSettings,
};

/// Trait for NAT mapping protocols
#[async_trait::async_trait]
pub trait MappingProtocol: Send + Sync + 'static {
    /// Initialize the protocol
    async fn initialize(settings: NatMappingSettings) -> Result<Box<Self>, AddressMapperError>
    where
        Self: Sized;

    /// Map a TCP address and return the external address
    async fn map_address(&mut self, address: &Multiaddr) -> Result<Multiaddr, AddressMapperError>;
}

pub struct ProtocolManager {
    nat_pmp: Box<dyn MappingProtocol>,
    upnp: Box<dyn MappingProtocol>,
}

impl ProtocolManager {
    pub async fn initialize(settings: NatMappingSettings) -> Result<Self, AddressMapperError> {
        let upnp = UpnpProtocol::initialize(settings).await?;
        let nat_pmp = NatPmp::initialize(settings).await?;
        Ok(Self { nat_pmp, upnp })
    }

    pub async fn try_map_address(
        &mut self,
        address: &Multiaddr,
    ) -> Result<Multiaddr, AddressMapperError> {
        if let Ok(external_address) = self.nat_pmp.map_address(address).await {
            tracing::info!(
                "Successfully mapped {} to {} using NAT-PMP",
                address,
                external_address
            );

            return Ok(external_address);
        }

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

#[cfg(test)]
mod real_gateway_tests {
    use tokio::runtime::Runtime;

    use super::*;

    #[test]
    #[ignore = "Runs against a real gateway"]
    fn map_address_via_protocol_manager_real_gateway() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let internal_addr: Multiaddr = "/ip4/192.168.1.35/tcp/12345"
                .to_owned()
                .parse()
                .expect("valid multiaddr");

            let settings = NatMappingSettings::default();

            let mut mgr = ProtocolManager::initialize(settings)
                .await
                .expect("initialize ProtocolManager");

            let external = mgr.try_map_address(&internal_addr).await.unwrap();

            let expected_external: Multiaddr =
                "/ip4/18.9.60.1/tcp/12345".to_owned().parse().unwrap();

            assert_eq!(external, expected_external);
        });
    }
}
