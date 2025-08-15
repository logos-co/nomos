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

    /// Tries to map an address and returns the external address
    async fn map_address(&mut self, address: &Multiaddr) -> Result<Multiaddr, AddressMapperError>;
}

pub struct ProtocolManager;

impl ProtocolManager {
    pub async fn try_map_address(
        settings: NatMappingSettings,
        address: &Multiaddr,
    ) -> Result<Multiaddr, AddressMapperError> {
        let mut nat_pmp = NatPmp::initialize(settings).await?;
        if let Ok(external_address) = nat_pmp.map_address(address).await {
            tracing::info!(
                "Successfully mapped {} to {} using NAT-PMP",
                address,
                external_address
            );

            return Ok(external_address);
        }

        let mut upnp = UpnpProtocol::initialize(settings).await?;
        let external_address = upnp.map_address(address).await?;
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

            let external = ProtocolManager::try_map_address(settings, &internal_addr)
                .await
                .expect("initialize ProtocolManager");

            let expected_external: Multiaddr =
                "/ip4/18.9.60.1/tcp/12345".to_owned().parse().unwrap();

            assert_eq!(external, expected_external);
        });
    }
}
