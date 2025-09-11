use std::{
    collections::{BTreeSet, HashMap},
    str::FromStr as _,
};

use nomos_core::sdp::{Locator, ServiceType};
use nomos_libp2p::{ed25519, Multiaddr};
use nomos_membership::{backends::mock::MockMembershipBackendSettings, MembershipServiceSettings};
use serde::{Deserialize, Serialize};

use crate::secret_key_to_provider_id;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeneralMembershipConfig {
    pub service_settings: MembershipServiceSettings<MockMembershipBackendSettings>,
}

#[must_use]
pub fn create_membership_configs(ids: &[[u8; 32]], ports: &[u16]) -> Vec<GeneralMembershipConfig> {
    let mut providers = HashMap::new();

    for (i, id) in ids.iter().enumerate() {
        let mut node_key_bytes = *id;
        let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
            .expect("Failed to generate secret key from bytes");
        let provider_id = secret_key_to_provider_id(node_key.clone());

        let listening_address =
            Multiaddr::from_str(&format!("/ip4/127.0.0.1/udp/{}/quic-v1", ports[i]))
                .expect("Failed to create multiaddr");

        let mut locs = BTreeSet::new();
        locs.insert(Locator::new(listening_address));

        providers.insert(provider_id, locs);
    }

    let mock_backend_settings = MockMembershipBackendSettings {
        session_sizes: HashMap::from([(ServiceType::DataAvailability, 4)]),
        session_zero_providers: HashMap::from([(ServiceType::DataAvailability, providers)]),
    };

    let config = GeneralMembershipConfig {
        service_settings: MembershipServiceSettings {
            backend: mock_backend_settings,
        },
    };

    ids.iter().map(|_| config.clone()).collect()
}

#[must_use]
pub fn create_empty_membership_configs(n_participants: usize) -> Vec<GeneralMembershipConfig> {
    let mut membership_configs = vec![];

    for _ in 0..n_participants {
        membership_configs.push(GeneralMembershipConfig {
            service_settings: MembershipServiceSettings {
                backend: MockMembershipBackendSettings {
                    session_sizes: HashMap::from([(ServiceType::DataAvailability, 4)]),
                    session_zero_providers: HashMap::new(),
                },
            },
        });
    }

    membership_configs
}
