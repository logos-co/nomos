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
            tracing::info!("Successfully mapped {address} to {external_address} using NAT-PMP");

            return Ok(external_address);
        }

        let mut upnp = UpnpProtocol::initialize(settings).await?;
        let external_address = upnp.map_address(address).await?;
        tracing::info!("Successfully mapped {address} to {external_address} using UPnP");

        Ok(external_address)
    }
}

#[cfg(test)]
mod real_gateway_tests {
    use rand::{thread_rng, Rng as _};

    use super::*;

    /// Run with: NAT_TEST_LOCAL_IP="192.168.1.100" cargo test
    /// map_address_via_protocol_manager_real_gateway -- --ignored
    #[tokio::test]
    #[ignore = "Runs against a real gateway"]
    async fn map_address_via_protocol_manager_real_gateway() {
        let local_ip = std::env::var("NAT_TEST_LOCAL_IP").expect("NAT_TEST_LOCAL_IP");
        let random_port: u64 = thread_rng().gen_range(10000..=64000);
        let internal_addr = format!("/ip4/{local_ip}/tcp/{random_port}");

        println!("Testing NAT mapping for internal address: {internal_addr}",);

        let internal_addr: Multiaddr = internal_addr.parse().expect("valid multiaddr");

        let settings = NatMappingSettings::default();
        let external = ProtocolManager::try_map_address(settings, &internal_addr)
            .await
            .expect("initialize ProtocolManager");

        println!("Successfully mapped {internal_addr} to {external}");
    }
}
