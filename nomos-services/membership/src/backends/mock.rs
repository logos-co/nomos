use std::collections::HashMap;

use nomos_sdp_core::{
    BlockNumber, DeclarationUpdate, FinalizedBlockEvent, ProviderInfo, ServiceType,
};

use super::{MembershipBackend, MembershipBackendError, Settings};
use crate::MembershipProviders;

pub struct MockMembershipBackendSettings {
    settings_per_service: HashMap<ServiceType, Settings>,
    initial_membership: HashMap<BlockNumber, MockMembershipEntry>,
}

type MockMembershipEntry = HashMap<ServiceType, HashMap<ProviderInfo, DeclarationUpdate>>;

pub struct MockMembershipBackend {
    settings: MockMembershipBackendSettings,
    membership: HashMap<BlockNumber, MockMembershipEntry>,
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

            settings,
        }
    }

    async fn get_providers_at(
        &self,
        service_type: ServiceType,
        block_number: BlockNumber,
    ) -> Result<MembershipProviders, MembershipBackendError> {
        let k = self
            .settings
            .settings_per_service
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
        let mut result = HashMap::new();

        let mut current_entry = self
            .membership
            .get(&self.latest_block_number)
            .cloned()
            .unwrap_or_default();

        for (provider_info, declaration_update) in update.updates {
            if !self
                .settings
                .settings_per_service
                .contains_key(&declaration_update.service_type)
            {
                continue;
            }

            let service_type = declaration_update.service_type;
            let service_data = current_entry.entry(service_type).or_default();
            service_data.insert(provider_info, declaration_update.clone());
            result.insert(service_type, HashMap::new());
        }

        self.latest_block_number = block_number;
        // todo: figure out pruning the old entries
        self.membership.insert(block_number, current_entry.clone());

        for (service_type, snapshot) in current_entry {
            result.insert(
                service_type,
                snapshot
                    .iter()
                    .map(|(provider_info, declaration_update)| {
                        (
                            provider_info.provider_id,
                            declaration_update.locators.clone(),
                        )
                    })
                    .collect(),
            );
        }

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
            .map_or_else(HashMap::new, |data| {
                data.iter()
                    .map(|(provider_info, declaration_update)| {
                        (
                            provider_info.provider_id,
                            declaration_update.locators.clone(),
                        )
                    })
                    .collect()
            })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use multiaddr::multiaddr;
    use nomos_sdp_core::{
        BlockNumber, DeclarationId, DeclarationUpdate, FinalizedBlockEvent, Locator, ProviderId,
        ProviderInfo, ServiceType,
    };

    use super::{
        MembershipBackend as _, MockMembershipBackend, MockMembershipBackendSettings, Settings,
    };

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

        let mut service_data = HashMap::new();
        service_data.insert(provider_info_1, declaration_update_1.clone());
        service_data.insert(provider_info_2, declaration_update_2.clone());

        let mut initial_entry1 = HashMap::new();
        initial_entry1.insert(service_type, service_data);

        let mut service_data_2 = HashMap::new();
        service_data_2.insert(provider_info_2, declaration_update_2.clone());

        let mut initial_entry2 = HashMap::new();
        initial_entry2.insert(service_type, service_data_2);

        let provider_info_3 = create_provider_info(3, 100);
        let declaration_update_3 = create_declaration_update(3, service_type, 3);

        let mut service_data_3 = HashMap::new();
        service_data_3.insert(provider_info_3, declaration_update_3.clone());

        let mut initial_entry3 = HashMap::new();
        initial_entry3.insert(service_type, service_data_3);

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
                (100, initial_entry1),
                (101, initial_entry2),
                (102, initial_entry3),
            ]),
        };

        let backend = MockMembershipBackend::init(settings);

        // (1st entry)
        // blocknumber 100 = 105 - k.historical_block_delta
        let result = backend.get_providers_at(service_type, 105).await.unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key(&provider_info_1.provider_id));
        assert!(result.contains_key(&provider_info_2.provider_id));
        assert_eq!(
            result.get(&provider_info_1.provider_id).unwrap(),
            &declaration_update_1.locators
        );
        assert_eq!(
            result.get(&provider_info_2.provider_id).unwrap(),
            &declaration_update_2.locators
        );

        // (second entry)
        // should be same as the first one since the same provider was added
        let result = backend.get_providers_at(service_type, 106).await.unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&provider_info_2.provider_id));
        assert_eq!(
            result.get(&provider_info_2.provider_id).unwrap(),
            &declaration_update_2.locators
        );

        // (third entry)
        let result = backend.get_providers_at(service_type, 107).await.unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&provider_info_3.provider_id));
        assert_eq!(
            result.get(&provider_info_3.provider_id).unwrap(),
            &declaration_update_3.locators
        );

        // latest one should be same as the one we just added
        let result = backend.get_latest_providers(service_type).await.unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&provider_info_3.provider_id));
        assert_eq!(
            result.get(&provider_info_3.provider_id).unwrap(),
            &declaration_update_3.locators
        );
    }

    #[tokio::test]
    async fn test_update() {
        let service_type = ServiceType::BlendNetwork;
        let mut settings_per_service = HashMap::new();
        let historical_delta = 5;
        settings_per_service.insert(
            service_type,
            Settings {
                historical_block_delta: historical_delta,
            },
        );

        let settings = MockMembershipBackendSettings {
            settings_per_service,
            initial_membership: HashMap::new(),
        };

        let mut backend = MockMembershipBackend::init(settings);

        // Create update data
        let provider_info_1 = create_provider_info(1, 5);
        let declaration_update_1 = create_declaration_update(1, service_type, 3);

        let updates = vec![(provider_info_1, declaration_update_1.clone())];

        // Test update with non-snapshot block
        let event = FinalizedBlockEvent {
            block_number: 5,
            updates,
        };

        let result = backend.update(event).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(backend.membership.len(), 1);
        let snapshot = result.get(&service_type).unwrap();
        assert_eq!(snapshot.len(), 1);
        assert!(snapshot.contains_key(&provider_info_1.provider_id));

        let provider_info_2 = create_provider_info(2, 6);
        let declaration_update_2 = create_declaration_update(2, service_type, 3);

        let updates = vec![(provider_info_2, declaration_update_2.clone())];

        let event = FinalizedBlockEvent {
            block_number: 6,
            updates,
        };

        let result = backend.update(event).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(backend.membership.len(), 2);
        let snapshot = result.get(&service_type).unwrap();
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.contains_key(&provider_info_2.provider_id));
        assert!(snapshot.contains_key(&provider_info_1.provider_id));
        assert_eq!(
            snapshot.get(&provider_info_1.provider_id).unwrap(),
            &declaration_update_1.locators
        );
        assert_eq!(
            snapshot.get(&provider_info_2.provider_id).unwrap(),
            &declaration_update_2.locators
        );

        // test latest, should be the same as the last one
        let result = backend.get_latest_providers(service_type).await.unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key(&provider_info_2.provider_id));
        assert!(result.contains_key(&provider_info_1.provider_id));
        assert_eq!(
            result.get(&provider_info_1.provider_id).unwrap(),
            &declaration_update_1.locators
        );
        assert_eq!(
            result.get(&provider_info_2.provider_id).unwrap(),
            &declaration_update_2.locators
        );

        // test get_providers_at to get the first entry
        let result = backend
            .get_providers_at(service_type, 5 + historical_delta)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&provider_info_1.provider_id));
        assert_eq!(
            result.get(&provider_info_1.provider_id).unwrap(),
            &declaration_update_1.locators
        );
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
        };

        let mut backend = MockMembershipBackend::init(settings);

        // unknown service type
        let provider_info = create_provider_info(1, 5);
        let declaration_update = create_declaration_update(1, unknown_service_type, 3);

        let updates = vec![(provider_info, declaration_update)];

        let event = FinalizedBlockEvent {
            block_number: 5,
            updates,
        };

        let result = backend.update(event).await.unwrap();
        assert_eq!(result.len(), 0);
    }
}
