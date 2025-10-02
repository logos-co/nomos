use std::{
    collections::{BTreeSet, HashMap},
    str::FromStr as _,
};

use nomos_core::sdp::{Locator, ServiceType};
use nomos_libp2p::{Multiaddr, ed25519};
use nomos_membership_service::{
    MembershipServiceSettings, backends::membership::MembershipBackendSettings,
};
use serde::{Deserialize, Serialize};

use crate::secret_key_to_provider_id;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeneralMembershipConfig {
    pub service_settings: MembershipServiceSettings<MembershipBackendSettings>,
}

pub struct MembershipNode {
    pub id: [u8; 32],
    pub da_port: Option<u16>,
    pub blend_port: Option<u16>,
}

#[must_use]
pub fn create_membership_configs(nodes: &[MembershipNode]) -> Vec<GeneralMembershipConfig> {
    let mut providers = HashMap::new();

    for node in nodes {
        let mut node_key_bytes = node.id;
        let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
            .expect("Failed to generate secret key from bytes");
        let provider_id = secret_key_to_provider_id(node_key.clone());

        if let Some(da_port) = node.da_port {
            let da_listening_address =
                Multiaddr::from_str(&format!("/ip4/127.0.0.1/udp/{da_port}/quic-v1"))
                    .expect("Failed to create multiaddr for DA");
            providers
                .entry(ServiceType::DataAvailability)
                .or_insert_with(HashMap::new)
                .insert(
                    provider_id,
                    BTreeSet::from([Locator::new(da_listening_address)]),
                );
        }

        if let Some(blend_port) = node.blend_port {
            let blend_listening_address =
                Multiaddr::from_str(&format!("/ip4/127.0.0.1/udp/{blend_port}/quic-v1"))
                    .expect("Failed to create multiaddr for Blend");

            providers
                .entry(ServiceType::BlendNetwork)
                .or_insert_with(HashMap::new)
                .insert(
                    provider_id,
                    BTreeSet::from([Locator::new(blend_listening_address)]),
                );
        }
    }

    let mock_backend_settings = MembershipBackendSettings {
        session_sizes: HashMap::from([
            (ServiceType::DataAvailability, 4),
            (ServiceType::BlendNetwork, 10),
        ]),
        session_zero_providers: providers,
    };

    let config = GeneralMembershipConfig {
        service_settings: MembershipServiceSettings {
            backend: mock_backend_settings,
        },
    };

    nodes.iter().map(|_| config.clone()).collect()
}
