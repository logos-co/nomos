use std::{
    collections::{BTreeSet, HashMap, HashSet},
    str::FromStr as _,
};

use nomos_core::sdp::{Locator, ServiceType};
use nomos_libp2p::{ed25519, Multiaddr};
use nomos_membership::{
    backends::{mock::MockMembershipBackendSettings, MembershipBackendServiceSettings},
    MembershipServiceSettings,
};
use serde::{Deserialize, Serialize};

use crate::secret_key_to_provider_id;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeneralMembershipConfig {
    pub service_settings: MembershipServiceSettings<MockMembershipBackendSettings>,
}

#[must_use]
pub fn create_membership_configs(ids: &[[u8; 32]], ports: &[u16]) -> Vec<GeneralMembershipConfig> {
    let settings_per_service = HashMap::from([(
        ServiceType::DataAvailability,
        MembershipBackendServiceSettings {
            historical_block_delta: 8,
        },
    )]);

    let mut provider_ids = vec![];
    let mut locators = HashMap::new();

    for (i, id) in ids.iter().enumerate() {
        let mut node_key_bytes = *id;
        let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
            .expect("Failed to generate secret key from bytes");

        let provider_id = secret_key_to_provider_id(node_key.clone());
        provider_ids.push(provider_id);

        let listening_address =
            Multiaddr::from_str(&format!("/ip4/127.0.0.1/udp/{}/quic-v1", ports[i]))
                .expect("Failed to create multiaddr");

        locators.insert(provider_id, Locator::new(listening_address));
    }

    let mut initial_membership = HashMap::new();
    let mut initial_locators_mapping = HashMap::new();
    let mut membership_entry = HashMap::new();
    let mut providers = HashSet::new();

    for provider_id in provider_ids {
        providers.insert(provider_id);
    }

    membership_entry.insert(ServiceType::DataAvailability, providers.clone());
    initial_membership.insert(0u64, membership_entry);

    for provider_id in providers {
        let mut locs = BTreeSet::new();
        if let Some(locator) = locators.get(&provider_id) {
            locs.insert(locator.clone());
        }
        initial_locators_mapping.insert(provider_id, locs);
    }

    let mock_backend_settings = MockMembershipBackendSettings {
        settings_per_service,
        initial_membership,
        initial_locators_mapping,
        latest_block_number: 0,
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

    let settings_per_service = HashMap::from([(
        ServiceType::DataAvailability,
        MembershipBackendServiceSettings {
            historical_block_delta: 8,
        },
    )]);

    for _ in 0..n_participants {
        membership_configs.push(GeneralMembershipConfig {
            service_settings: MembershipServiceSettings {
                backend: MockMembershipBackendSettings {
                    settings_per_service: settings_per_service.clone(),
                    initial_membership: HashMap::new(),
                    initial_locators_mapping: HashMap::new(),
                    latest_block_number: 0,
                },
            },
        });
    }

    membership_configs
}
