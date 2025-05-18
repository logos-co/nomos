use std::{
    collections::{HashMap, HashSet},
    vec,
};

use nomos_sdp_core::{BlockNumber, FinalizedBlockEvent, Locator, ProviderId, ServiceType};

use super::{MembershipBackend, MembershipBackendError, Settings};
use crate::MembershipProviders;

pub struct MockMembershipBackendSettings {
    settings_per_service: HashMap<ServiceType, Settings>,
    initial_membership: HashMap<BlockNumber, MockMembershipEntry>,
    initial_locators_mapping: HashMap<ProviderId, Vec<Locator>>,
}

type MockMembershipEntry = HashMap<ServiceType, HashSet<ProviderId>>;

//todo: always use new locators even for old
// so we will need a mapping providerInfo -> new locators
// also FinalizedBlockEvent needs state, so we can remnov

pub struct MockMembershipBackend {
    settings: HashMap<ServiceType, Settings>,
    membership: HashMap<BlockNumber, MockMembershipEntry>,

    // this is a mapping of providerId -> locators
    // this is used to get the latest locators for a provider
    locators_mapping: HashMap<ProviderId, Vec<Locator>>,
    latest_block_number: BlockNumber,
}

#[async_trait::async_trait]
impl MembershipBackend for MockMembershipBackend {
    type Settings = MockMembershipBackendSettings;
    fn init(settings: MockMembershipBackendSettings) -> Self {
        Self {
            membership: settings.initial_membership.clone(),
            latest_block_number: settings
                .initial_membership
                .keys()
                .copied()
                .max()
                .unwrap_or(0),
            settings: settings.settings_per_service,
            locators_mapping: settings.initial_locators_mapping,
        }
    }

    async fn get_providers_at(
        &self,
        service_type: ServiceType,
        block_number: BlockNumber,
    ) -> Result<MembershipProviders, MembershipBackendError> {
        let k = self
            .settings
            .get(&service_type)
            .ok_or_else(|| MembershipBackendError::Other("Service type not found".into()))?;
        let index = block_number.saturating_sub(k.historical_block_delta);
        return Ok(self.get_snapshot(index, service_type));
    }

    async fn get_latest_providers(
        &self,
        service_type: ServiceType,
    ) -> Result<MembershipProviders, MembershipBackendError> {
        return Ok(self.get_snapshot(self.latest_block_number, service_type));
    }

    async fn update(
        &mut self,
        update: FinalizedBlockEvent,
    ) -> Result<HashMap<ServiceType, MembershipProviders>, MembershipBackendError> {
        let block_number = update.block_number;

        let mut latest_entry = self
            .membership
            .get(&self.latest_block_number)
            .cloned()
            .unwrap_or_default();

        let mut updated_service_types = vec![];

        for (service_type, provider_id, state, locators) in update.updates {
            if !self.settings.contains_key(&service_type) {
                continue;
            }

            updated_service_types.push(service_type);

            let service_data = latest_entry.entry(service_type).or_default();

            match state {
                nomos_sdp_core::state::ProviderState::Active(_initstate) => {
                    self.locators_mapping.insert(provider_id, locators.clone());
                    service_data.insert(provider_id);
                }
                nomos_sdp_core::state::ProviderState::Inactive(_)
                | nomos_sdp_core::state::ProviderState::Withdrawn(_) => {
                    service_data.remove(&provider_id);
                    self.locators_mapping.remove(&provider_id);
                }
            }
        }

        self.latest_block_number = block_number;
        self.membership.insert(block_number, latest_entry.clone());

        let result = updated_service_types
            .into_iter()
            .map(|service_type| {
                let snapshot = self.get_snapshot(block_number, service_type);
                (service_type, snapshot)
            })
            .collect();

        Ok(result)
    }
}

impl MockMembershipBackend {
    fn get_snapshot(
        &self,
        block_number: BlockNumber,
        service_type: ServiceType,
    ) -> MembershipProviders {
        self.membership
            .get(&block_number)
            .and_then(|entry| entry.get(&service_type))
            .map(|snapshot| {
                snapshot
                    .iter()
                    .map(|provider_id| {
                        (
                            *provider_id,
                            self.locators_mapping
                                .get(provider_id)
                                .cloned()
                                .unwrap_or_default(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use multiaddr::multiaddr;
    use nomos_sdp_core::{
        state::{ActiveState, ProviderState, WithdrawnState},
        BlockNumber, DeclarationId, DeclarationUpdate, FinalizedBlockEvent, Locator, ProviderId,
        ProviderInfo, ServiceType,
    };

    use super::{
        MembershipBackend as _, MockMembershipBackend, MockMembershipBackendSettings, Settings,
    };
    use crate::MembershipProviders;

    // Helper function to create ProviderId with specified bytes
    fn create_provider_id(seed: u8) -> ProviderId {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        ProviderId(bytes)
    }

    // Helper function to create DeclarationId with specified bytes
    fn create_declaration_id(seed: u8) -> DeclarationId {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        DeclarationId(bytes)
    }

    // Helper function to create ProviderInfo
    fn create_provider_info(seed: u8, block_number: BlockNumber) -> ProviderInfo {
        ProviderInfo::new(
            block_number,
            create_provider_id(seed),
            create_declaration_id(seed),
        )
    }

    fn create_locator(seed: u8) -> Locator {
        Locator {
            addr: multiaddr!(Ip4([10, 0, 0, seed]), Udp(8000u16 + u16::from(seed))),
        }
    }

    // Helper function to create DeclarationUpdate
    fn create_declaration_update(
        seed: u8,
        service_type: ServiceType,
        num_locators: usize,
    ) -> DeclarationUpdate {
        let locators = (0..num_locators)
            .map(|i| create_locator(seed + i as u8))
            .collect();

        DeclarationUpdate {
            declaration_id: create_declaration_id(seed),
            provider_id: create_provider_id(seed),
            service_type,
            locators,
        }
    }

    #[tokio::test]
    async fn test_get_snapshot_at_empty() {
        let service_type = ServiceType::BlendNetwork;
        let mut settings_per_service = HashMap::new();
        settings_per_service.insert(
            service_type,
            Settings {
                historical_block_delta: 10,
            },
        );

        let settings = MockMembershipBackendSettings {
            settings_per_service,
            initial_membership: HashMap::new(),
            initial_locators_mapping: HashMap::new(),
        };

        let backend = MockMembershipBackend::init(settings);

        // Test with empty membership
        let result = backend.get_providers_at(service_type, 5).await.unwrap();
        assert_eq!(result.len(), 0);
    }

    #[tokio::test]
    async fn test_get_snapshot_at_with_data() {
        let service_type = ServiceType::BlendNetwork;

        let provider_info_1 = create_provider_info(1, 100);
        let provider_info_2 = create_provider_info(2, 100);

        let declaration_update_1 = create_declaration_update(1, service_type, 3);
        let declaration_update_2 = create_declaration_update(2, service_type, 3);

        let provider_info_3 = create_provider_info(3, 100);
        let declaration_update_3 = create_declaration_update(3, service_type, 3);

        let mut settings_per_service = HashMap::new();
        settings_per_service.insert(
            service_type,
            Settings {
                historical_block_delta: 5,
            },
        );

        let settings = MockMembershipBackendSettings {
            settings_per_service,
            initial_membership: HashMap::from([
                (
                    100,
                    HashMap::from([(service_type, HashSet::from([provider_info_1.provider_id]))]),
                ),
                (
                    101,
                    HashMap::from([(
                        service_type,
                        HashSet::from([provider_info_1.provider_id, provider_info_2.provider_id]),
                    )]),
                ),
                (
                    102,
                    HashMap::from([(
                        service_type,
                        HashSet::from([provider_info_1.provider_id, provider_info_3.provider_id]),
                    )]),
                ),
            ]),
            initial_locators_mapping: HashMap::from([
                (
                    provider_info_1.provider_id,
                    declaration_update_1.locators.clone(),
                ),
                (
                    provider_info_2.provider_id,
                    declaration_update_2.locators.clone(),
                ),
                (
                    provider_info_3.provider_id,
                    declaration_update_3.locators.clone(),
                ),
            ]),
        };

        let backend = MockMembershipBackend::init(settings);

        // (1st entry)
        // blocknumber 100 = 105 - k.historical_block_delta
        let result = backend.get_providers_at(service_type, 105).await.unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&provider_info_1.provider_id));
        assert_eq!(
            result.get(&provider_info_1.provider_id).unwrap(),
            &declaration_update_1.locators
        );

        // (second entry)
        // should have 1st and 2nd
        let result = backend.get_providers_at(service_type, 106).await.unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key(&provider_info_1.provider_id));
        assert!(result.contains_key(&provider_info_2.provider_id));

        assert_eq!(
            result.get(&provider_info_2.provider_id).unwrap(),
            &declaration_update_2.locators
        );
        assert_eq!(
            result.get(&provider_info_1.provider_id).unwrap(),
            &declaration_update_1.locators
        );

        // (third entry)
        // should have 1st and 3rd
        let result = backend.get_providers_at(service_type, 107).await.unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key(&provider_info_1.provider_id));
        assert!(result.contains_key(&provider_info_3.provider_id));
        assert_eq!(
            result.get(&provider_info_3.provider_id).unwrap(),
            &declaration_update_3.locators
        );
        assert_eq!(
            result.get(&provider_info_3.provider_id).unwrap(),
            &declaration_update_3.locators
        );

        // latest one should be same as the one we just added
        let result = backend.get_latest_providers(service_type).await.unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key(&provider_info_1.provider_id));
        assert!(result.contains_key(&provider_info_3.provider_id));
        assert_eq!(
            result.get(&provider_info_1.provider_id).unwrap(),
            &declaration_update_1.locators
        );
        assert_eq!(
            result.get(&provider_info_3.provider_id).unwrap(),
            &declaration_update_3.locators
        );
    }

    #[tokio::test]
    async fn test_update() {
        let service_type = ServiceType::BlendNetwork;
        let mut settings_per_service = HashMap::new();
        settings_per_service.insert(
            service_type,
            Settings {
                historical_block_delta: 5,
            },
        );
        let settings = MockMembershipBackendSettings {
            settings_per_service,
            initial_membership: HashMap::new(),
            initial_locators_mapping: HashMap::new(),
        };
        let mut backend = MockMembershipBackend::init(settings);

        let provider_info_1 = create_provider_info(1, 100);
        let declaration_update_1 = create_declaration_update(1, service_type, 3);
        let updates = vec![(
            service_type,
            provider_info_1.provider_id,
            ProviderState::Active(ActiveState(provider_info_1)),
            declaration_update_1.locators.clone(),
        )];
        let event = FinalizedBlockEvent {
            block_number: 100,
            updates,
        };

        let result = backend.update(event).await.unwrap();
        assert_update_result(
            &result,
            service_type,
            &[(provider_info_1.provider_id, &declaration_update_1.locators)],
        );

        let provider_info_2 = create_provider_info(2, 100);
        let declaration_update_2 = create_declaration_update(2, service_type, 3);
        let updates = vec![(
            service_type,
            provider_info_2.provider_id,
            ProviderState::Active(ActiveState(provider_info_2)),
            declaration_update_2.locators.clone(),
        )];
        let event = FinalizedBlockEvent {
            block_number: 101,
            updates,
        };

        let result = backend.update(event).await.unwrap();
        assert_update_result(
            &result,
            service_type,
            &[
                (provider_info_1.provider_id, &declaration_update_1.locators),
                (provider_info_2.provider_id, &declaration_update_2.locators),
            ],
        );

        let provider_info_3 = create_provider_info(3, 100);
        let declaration_update_3 = create_declaration_update(3, service_type, 3);
        let updates = vec![
            (
                service_type,
                provider_info_2.provider_id,
                ProviderState::Withdrawn(WithdrawnState(provider_info_2)),
                Vec::new(),
            ),
            (
                service_type,
                provider_info_3.provider_id,
                ProviderState::Active(ActiveState(provider_info_3)),
                declaration_update_3.locators.clone(),
            ),
        ];
        let event = FinalizedBlockEvent {
            block_number: 102,
            updates,
        };

        let result = backend.update(event).await.unwrap();
        assert_update_result(
            &result,
            service_type,
            &[
                (provider_info_1.provider_id, &declaration_update_1.locators),
                (provider_info_3.provider_id, &declaration_update_3.locators),
            ],
        );

        let result = backend.get_latest_providers(service_type).await.unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key(&provider_info_1.provider_id));
        assert!(result.contains_key(&provider_info_3.provider_id));
        assert_eq!(
            result.get(&provider_info_1.provider_id).unwrap(),
            &declaration_update_1.locators
        );
        assert_eq!(
            result.get(&provider_info_3.provider_id).unwrap(),
            &declaration_update_3.locators
        );
    }

    fn assert_update_result(
        result: &HashMap<ServiceType, MembershipProviders>,
        service_type: ServiceType,
        expected_providers: &[(ProviderId, &Vec<Locator>)],
    ) {
        assert_eq!(
            result.len(),
            1,
            "Result should contain exactly one service type"
        );
        assert!(
            result.contains_key(&service_type),
            "Result should contain the expected service type"
        );

        let providers = result.get(&service_type).unwrap();

        // Only check providers that were part of this update
        for (provider_id, expected_locators) in expected_providers {
            assert!(
                providers.contains_key(provider_id),
                "Providers map should contain provider {provider_id:?}"
            );
            assert_eq!(
                providers.get(provider_id).unwrap(),
                *expected_locators,
                "Locators for provider {provider_id:?} do not match expected",
            );
        }
    }

    #[tokio::test]
    async fn test_update_missing_service_type() {
        let service_type = ServiceType::BlendNetwork;
        let unknown_service_type = ServiceType::DataAvailability;

        let mut settings_per_service = HashMap::new();
        settings_per_service.insert(
            service_type,
            Settings {
                historical_block_delta: 5,
            },
        );

        let settings = MockMembershipBackendSettings {
            settings_per_service,
            initial_membership: HashMap::new(),
            initial_locators_mapping: HashMap::new(),
        };

        let mut backend = MockMembershipBackend::init(settings);

        // unknown service type
        let provider_info = create_provider_info(1, 5);
        let declaration_update = create_declaration_update(1, unknown_service_type, 3);

        let updates = vec![(
            declaration_update.service_type,
            provider_info.provider_id,
            ProviderState::Active(ActiveState(provider_info)),
            declaration_update.locators,
        )];

        let event = FinalizedBlockEvent {
            block_number: 5,
            updates,
        };

        let result = backend.update(event).await.unwrap();
        assert_eq!(result.len(), 0);
    }
}
