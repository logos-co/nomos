use std::collections::HashMap;

use nomos_sdp_core::{DeclarationUpdate, FinalizedBlockEvent, ProviderInfo, ServiceType};

use super::{MembershipBackend, MembershipEntry, SnapshotSettings};

pub struct MockMembershipBackendSettings {
    pub settings_per_service: HashMap<ServiceType, SnapshotSettings>,
    pub initial_membership: Vec<MembershipEntry>,
}

pub struct MockMembershipBackend {
    settings: MockMembershipBackendSettings,
    // todo: storage trait instead of deque in memory
    membership: Vec<MembershipEntry>,
    current_data: HashMap<ServiceType, HashMap<ProviderInfo, DeclarationUpdate>>,
}

#[async_trait::async_trait]
impl MembershipBackend for MockMembershipBackend {
    type Settings = MockMembershipBackendSettings;
    fn init(settings: MockMembershipBackendSettings) -> Self {
        Self {
            membership: settings.initial_membership.clone(),
            settings,
            current_data: HashMap::new(),
        }
    }

    async fn get_snapshot_at(
        &self,
        service_type: ServiceType,
        index: i32,
    ) -> Result<HashMap<ProviderInfo, DeclarationUpdate>, overwatch::DynError> {
        let len = self.membership.len();

        if len == 0 {
            return Ok(HashMap::default());
        }

        let actual_index = if index >= 0 {
            if index as usize >= len {
                return Ok(HashMap::default());
            }
            len - 1 - (index as usize)
        } else {
            let positive_index = (-index) as usize;
            if positive_index > len {
                return Ok(HashMap::default());
            }
            len - positive_index
        };

        let entry = &self.membership[actual_index];

        Ok(entry.data.get(&service_type).cloned().unwrap_or_default())
    }

    async fn update(&mut self, update: FinalizedBlockEvent) -> Result<(), overwatch::DynError> {
        for (provider_info, declaration_update) in update.updates {
            let service_type = declaration_update.service_type;
            let service_map = self.current_data.entry(service_type).or_default();
            // todo: figure out based on pruning strategy
            service_map.insert(provider_info, declaration_update);
        }

        for service_type in self.current_data.keys() {
            if let Some(snapshot_settings) = self.settings.settings_per_service.get(service_type) {
                match snapshot_settings {
                    SnapshotSettings::Block(snapshot_period) => {
                        if update.block_number % *snapshot_period as u64 == 0 {
                            let entry = MembershipEntry {
                                block_number: update.block_number,
                                data: self.current_data.clone(),
                            };
                            self.membership.push(entry);
                        }
                    }
                }
            } else {
                return Err(overwatch::DynError::from(format!(
                    "Service type {service_type:?} not found in settings"
                )));
            }
        }
        Ok(())
    }
}
