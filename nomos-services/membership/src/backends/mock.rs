use std::collections::HashMap;

use nomos_sdp_core::{DeclarationUpdate, FinalizedBlockEvent, ProviderInfo, ServiceType};

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

pub struct MockMembershipBackend {
    settings: MockMembershipBackendSettings,
    // todo: storage trait instead of deque in memory
    membership: Vec<MockMembershipEntry>,
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
    ) -> Result<MembershipSnapshot, MembershipBackendError> {
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

        let snapshot = self.membership[actual_index]
            .data
            .get(&service_type)
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|(provider_info, declaration_update)| {
                (
                    provider_info.provider_id,
                    declaration_update.locators.clone(),
                )
            })
            .collect();

        Ok(snapshot)
    }

    async fn update(
        &mut self,
        update: FinalizedBlockEvent,
    ) -> Result<HashMap<ServiceType, MembershipSnapshot>, MembershipBackendError> {
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
                            let entry = MockMembershipEntry {
                                data: self.current_data.clone(),
                            };
                            self.membership.push(entry);
                        }
                    }
                }
            } else {
                return Err(MembershipBackendError::Other(overwatch::DynError::from(
                    format!("Service type {service_type:?} not found in settings"),
                )));
            }
        }

        let snapshots = self
            .current_data
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
            .collect();

        Ok(snapshots)
    }
}
