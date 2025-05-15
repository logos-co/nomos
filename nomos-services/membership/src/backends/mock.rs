use std::collections::HashMap;

use nomos_sdp_core::{
    DeclarationUpdate, FinalizedBlockEvent, Locator, ProviderId, ProviderInfo, ServiceType,
};

use super::{MembershipBackend, MembershipBackendError, SnapshotSettings};
use crate::MembershipSnapshot;

pub struct MockMembershipBackendSettings {
    settings_per_service: HashMap<ServiceType, SnapshotSettings>,
    initial_membership: Vec<MockMembershipEntry>,
}

#[derive(Debug, Clone)]
struct MockMembershipEntry {
    pub data: HashMap<ServiceType, HashMap<ProviderInfo, DeclarationUpdate>>,
}

#[derive(Debug, Clone, Default)]
struct MockCurrentSnapshot {
    data: HashMap<ServiceType, HashMap<ProviderInfo, DeclarationUpdate>>,
    block_number: u64,
}

pub struct MockMembershipBackend {
    settings: MockMembershipBackendSettings,
    membership: Vec<MockMembershipEntry>,
    current_data: MockCurrentSnapshot,
}

#[async_trait::async_trait]
impl MembershipBackend for MockMembershipBackend {
    type Settings = MockMembershipBackendSettings;
    fn init(settings: MockMembershipBackendSettings) -> Self {
        Self {
            membership: settings.initial_membership.clone(),
            settings,
            current_data: MockCurrentSnapshot::default(),
        }
    }

    async fn get_snapshot_at(
        &self,
        service_type: ServiceType,
        index: i32,
    ) -> Result<MembershipSnapshot, MembershipBackendError> {
        // index 0 gets latest snapshot
        if index == 0 {
            let snapshot = self.get_latest_snapshots().get(&service_type).cloned();
            return Ok(snapshot.unwrap_or_default());
        }

        let len = self.membership.len();
        if len == 0 {
            return Ok(HashMap::default());
        }

        let actual_index = if index > 0 {
            // Positive index: absolute position in the past
            let idx = index as usize - 1;
            if idx >= len {
                return Ok(HashMap::default());
            }

            idx
        } else {
            // Negative index: relative to latest
            let index = self.membership.len() as i32 + index;
            if index < 0 {
                return Ok(HashMap::default());
            }
            index as usize
        };

        let snapshot = self.get_snapshot(actual_index, service_type);
        Ok(snapshot)
    }

    async fn update(
        &mut self,
        update: FinalizedBlockEvent,
    ) -> Result<HashMap<ServiceType, MembershipSnapshot>, MembershipBackendError> {
        let mut result = HashMap::new();
        let mut new_snapshot = MockMembershipEntry {
            data: HashMap::new(),
        };
        let mut should_create_snapshot = false;

        for service_type in self.current_data.data.keys() {
            if let Some(snapshot_settings) = self.settings.settings_per_service.get(service_type) {
                match snapshot_settings {
                    SnapshotSettings::Block(snapshot_period) => {
                        let current_period =
                            self.current_data.block_number / *snapshot_period as u64;
                        let new_period = update.block_number / *snapshot_period as u64;

                        if (new_period > current_period)
                            || (update.block_number % *snapshot_period as u64 == 0)
                        {
                            should_create_snapshot = true;
                            new_snapshot.data.insert(
                                *service_type,
                                self.current_data
                                    .data
                                    .get(service_type)
                                    .cloned()
                                    .unwrap_or_default(),
                            );
                        }
                    }
                }
            } else {
                return Err(MembershipBackendError::MockBackendError(
                    overwatch::DynError::from(format!(
                        "Service type {service_type:?} not found in settings"
                    )),
                ));
            }
        }

        // Update current data
        self.current_data.block_number = update.block_number;
        for (provider_info, declaration_update) in update.updates {
            let service_type = declaration_update.service_type;
            let service_map = self.current_data.data.entry(service_type).or_default();
            service_map.insert(provider_info, declaration_update);
        }

        // If no snapshot needed, just return
        if !should_create_snapshot {
            return Ok(result);
        }

        // Add the new snapshot and populate result
        self.membership.push(new_snapshot.clone());
        for service_type in new_snapshot.data.keys() {
            result.insert(*service_type, self.get_snapshot(0, *service_type));
        }

        Ok(result)
    }
}

impl MockMembershipBackend {
    fn get_snapshot(
        &self,
        index: usize,
        service_type: ServiceType,
    ) -> HashMap<ProviderId, Vec<Locator>> {
        self.membership[index]
            .data
            .get(&service_type)
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

    fn get_latest_snapshots(&self) -> HashMap<ServiceType, MembershipSnapshot> {
        self.current_data
            .data
            .iter()
            .map(|(service_type, data)| {
                (
                    *service_type,
                    data.iter()
                        .map(|(provider_info, declaration_update)| {
                            (
                                provider_info.provider_id,
                                declaration_update.locators.clone(),
                            )
                        })
                        .collect(),
                )
            })
            .collect()
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
        MembershipBackend as _, MockMembershipBackend, MockMembershipBackendSettings,
        MockMembershipEntry, SnapshotSettings,
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
        settings_per_service.insert(service_type, SnapshotSettings::Block(10));

        let settings = MockMembershipBackendSettings {
            settings_per_service,
            initial_membership: vec![],
        };

        let backend = MockMembershipBackend::init(settings);

        // Test with empty membership
        let result = backend.get_snapshot_at(service_type, 0).await.unwrap();
        assert_eq!(result.len(), 0);

        let result = backend.get_snapshot_at(service_type, 1).await.unwrap();
        assert_eq!(result.len(), 0);

        let result = backend.get_snapshot_at(service_type, -1).await.unwrap();
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

        let mut initial_data = HashMap::new();
        initial_data.insert(service_type, service_data);

        let entry1 = MockMembershipEntry { data: initial_data };

        let mut service_data_2 = HashMap::new();
        service_data_2.insert(provider_info_2, declaration_update_2.clone());

        let mut entry2_data = HashMap::new();
        entry2_data.insert(service_type, service_data_2);

        let entry2 = MockMembershipEntry { data: entry2_data };

        let provider_info_3 = create_provider_info(3, 100);
        let declaration_update_3 = create_declaration_update(3, service_type, 3);

        let mut service_data_3 = HashMap::new();
        service_data_3.insert(provider_info_3, declaration_update_3.clone());

        let mut entry3_data = HashMap::new();
        entry3_data.insert(service_type, service_data_3);

        let entry3 = MockMembershipEntry { data: entry3_data };

        let mut settings_per_service = HashMap::new();
        settings_per_service.insert(service_type, SnapshotSettings::Block(10));

        let settings = MockMembershipBackendSettings {
            settings_per_service,
            initial_membership: vec![entry1, entry2, entry3],
        };

        let mut backend = MockMembershipBackend::init(settings);

        // Update current data manually to simulate latest state
        let provider_info_4 = create_provider_info(4, 100);
        let declaration_update_4 = create_declaration_update(4, service_type, 3);

        let mut current_service_data = HashMap::new();
        current_service_data.insert(provider_info_4, declaration_update_4.clone());
        backend
            .current_data
            .data
            .insert(service_type, current_service_data);

        // (latest snapshot)
        let result = backend.get_snapshot_at(service_type, 0).await.unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&provider_info_4.provider_id));
        assert_eq!(
            result.get(&provider_info_4.provider_id).unwrap(),
            &declaration_update_4.locators
        );

        // (1 = first entry)
        let result = backend.get_snapshot_at(service_type, 1).await.unwrap();
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

        // (2 = second entry)
        let result = backend.get_snapshot_at(service_type, 2).await.unwrap();
        assert!(!result.contains_key(&provider_info_1.provider_id));
        assert!(result.contains_key(&provider_info_2.provider_id));

        // (3 = third entry)
        let result = backend.get_snapshot_at(service_type, 3).await.unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&provider_info_3.provider_id));
        assert_eq!(
            result.get(&provider_info_3.provider_id).unwrap(),
            &declaration_update_3.locators
        );

        // (4 > entries.len())
        let result = backend.get_snapshot_at(service_type, 4).await.unwrap();
        assert_eq!(result.len(), 0);

        // (-1 = latest historical entry)
        let result = backend.get_snapshot_at(service_type, -1).await.unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&provider_info_3.provider_id));
        assert_eq!(
            result.get(&provider_info_3.provider_id).unwrap(),
            &declaration_update_3.locators
        );

        // (-2 = second latest historical entry)
        let result = backend.get_snapshot_at(service_type, -2).await.unwrap();
        assert!(!result.contains_key(&provider_info_1.provider_id));
        assert!(result.contains_key(&provider_info_2.provider_id));

        // (-3 = third latest historical entry)
        let result = backend.get_snapshot_at(service_type, -3).await.unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key(&provider_info_1.provider_id));
        assert!(result.contains_key(&provider_info_2.provider_id));

        // negative index out of bounds
        let result = backend.get_snapshot_at(service_type, -4).await.unwrap();
        assert_eq!(result.len(), 0);
    }

    #[tokio::test]
    async fn test_update() {
        let service_type = ServiceType::BlendNetwork;
        let mut settings_per_service = HashMap::new();
        settings_per_service.insert(service_type, SnapshotSettings::Block(5));

        let settings = MockMembershipBackendSettings {
            settings_per_service,
            initial_membership: vec![],
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
        assert_eq!(result.len(), 0);
        assert_eq!(backend.membership.len(), 0);

        // Create another update at snapshot block
        let provider_info_2 = create_provider_info(2, 10);
        let declaration_update_2 = create_declaration_update(2, service_type, 3);

        let updates = vec![(provider_info_2, declaration_update_2.clone())];

        let event = FinalizedBlockEvent {
            block_number: 10, // This should trigger a snapshot
            updates,
        };

        let result = backend.update(event).await.unwrap();

        // Verify current data was updated
        assert_eq!(result.len(), 1);
        let snapshot = result.get(&service_type).unwrap();
        assert_eq!(snapshot.len(), 1); // should contain only the one saved in the snapshot
        assert!(snapshot.contains_key(&provider_info_1.provider_id));
        assert_eq!(
            snapshot.get(&provider_info_1.provider_id).unwrap(),
            &declaration_update_1.locators
        );

        // Verify snapshot was created
        assert_eq!(backend.membership.len(), 1);

        // Test current after updates
        let snapshot_result = backend.get_snapshot_at(service_type, 0).await.unwrap();
        assert_eq!(snapshot_result.len(), 2);
        assert!(snapshot_result.contains_key(&provider_info_1.provider_id));
        assert!(snapshot_result.contains_key(&provider_info_2.provider_id));

        assert_eq!(
            snapshot_result.get(&provider_info_1.provider_id).unwrap(),
            &declaration_update_1.locators
        );
        assert_eq!(
            snapshot_result.get(&provider_info_2.provider_id).unwrap(),
            &declaration_update_2.locators
        );

        // Test get_snapshot_at with index = 1 (should be the snapshot we created)
        let snapshot = backend.get_snapshot_at(service_type, 1).await.unwrap();
        assert_eq!(snapshot.len(), 1);
    }

    #[tokio::test]
    async fn test_update_missing_service_type() {
        let service_type = ServiceType::BlendNetwork;
        let unknown_service_type = ServiceType::DataAvailability;

        let mut settings_per_service = HashMap::new();
        settings_per_service.insert(service_type, SnapshotSettings::Block(10));

        let settings = MockMembershipBackendSettings {
            settings_per_service,
            initial_membership: vec![],
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

        let result = backend.update(event).await;
        assert!(result.is_err());
    }
}
