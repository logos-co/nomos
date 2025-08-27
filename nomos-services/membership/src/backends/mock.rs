use std::{
    collections::{BTreeSet, HashMap, HashSet},
    str,
};

use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{FinalizedBlockEvent, FinalizedBlockEventUpdate, Locator, ProviderId, ServiceType},
};
use serde::{Deserialize, Serialize};

use super::{MembershipBackend, MembershipBackendError};
use crate::{backends::NewSesssion, MembershipProviders};

type MockMembershipEntry = HashMap<ServiceType, HashSet<ProviderId>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockMembershipBackendSettings {
    pub session_size_blocks: u32,
    pub session_zero_membership: MockMembershipEntry,
    pub session_zero_locators_mapping: HashMap<ServiceType, HashMap<ProviderId, BTreeSet<Locator>>>,
}

pub struct MockMembershipBackend {
    // Only store the latest completed session
    active_session_id: SessionNumber,
    active_session_membership: MockMembershipEntry,
    active_session_locators: HashMap<ServiceType, HashMap<ProviderId, BTreeSet<Locator>>>,

    // Current session state
    latest_block_number: BlockNumber,
    session_size: u32,

    // In-flight session being formed
    forming_session_id: SessionNumber,
    forming_session_updates: MockMembershipEntry,
    forming_session_locators: HashMap<ServiceType, HashMap<ProviderId, BTreeSet<Locator>>>,
}

#[async_trait::async_trait]
impl MembershipBackend for MockMembershipBackend {
    type Settings = MockMembershipBackendSettings;
    fn init(settings: MockMembershipBackendSettings) -> Self {
        Self {
            active_session_id: 0,
            active_session_membership: settings.session_zero_membership.clone(),
            active_session_locators: settings.session_zero_locators_mapping.clone(),
            latest_block_number: 0,
            session_size: settings.session_size_blocks,
            // Start forming session 1 immediately as session 0 is seeded
            forming_session_id: 1,
            forming_session_updates: settings.session_zero_membership,
            forming_session_locators: settings.session_zero_locators_mapping,
        }
    }

    async fn get_latest_providers(
        &self,
        service_type: ServiceType,
    ) -> Result<MembershipProviders, MembershipBackendError> {
        Ok(self.get_active_session_snapshot(service_type))
    }

    async fn update(
        &mut self,
        update: FinalizedBlockEvent,
    ) -> Result<NewSesssion, MembershipBackendError> {
        let block_number = update.block_number;

        tracing::debug!(
            "Updating membership: {:?}, block {}, latest_block_number: {}",
            update,
            block_number,
            self.latest_block_number
        );

        if block_number <= self.latest_block_number {
            return Err(MembershipBackendError::BlockFromPast);
        }

        // Apply updates to forming session
        for FinalizedBlockEventUpdate {
            service_type,
            provider_id,
            state,
            locators,
        } in update.updates
        {
            let service_data = self
                .forming_session_updates
                .entry(service_type)
                .or_default();

            match state {
                nomos_core::sdp::FinalizedDeclarationState::Active => {
                    self.forming_session_locators
                        .entry(service_type)
                        .or_default()
                        .insert(provider_id, locators);

                    service_data.insert(provider_id);
                }
                nomos_core::sdp::FinalizedDeclarationState::Inactive
                | nomos_core::sdp::FinalizedDeclarationState::Withdrawn => {
                    service_data.remove(&provider_id);
                    self.forming_session_locators
                        .entry(service_type)
                        .or_default()
                        .remove(&provider_id);
                }
            }
        }

        self.latest_block_number = block_number;

        let next_session_id = self.block_to_session(block_number + 1);

        // Check if forming session is complete
        if self.forming_session_id <= next_session_id {
            // Move forming session to active
            self.active_session_id = self.forming_session_id;
            self.active_session_membership = self.forming_session_updates.clone();
            self.active_session_locators = self.forming_session_locators.clone();

            // Prepare result
            let result = self
                .active_session_membership
                .keys()
                .map(|service_type| {
                    (
                        *service_type,
                        self.get_active_session_snapshot(*service_type),
                    )
                })
                .collect();

            // Start next session
            self.forming_session_id += 1;
            self.forming_session_updates = self.active_session_membership.clone();
            self.forming_session_locators = self.active_session_locators.clone();

            Ok(Some(result))
        } else {
            // Session still forming
            Ok(None)
        }
    }
}

impl MockMembershipBackend {
    const fn block_to_session(&self, block_number: BlockNumber) -> SessionNumber {
        block_number / (self.session_size as BlockNumber)
    }

    fn get_active_session_snapshot(&self, service_type: ServiceType) -> MembershipProviders {
        let snapshot = self
            .active_session_membership
            .get(&service_type)
            .map(|providers| {
                providers
                    .iter()
                    .map(|pid| {
                        let locs = self
                            .active_session_locators
                            .get(&service_type)
                            .and_then(|m| m.get(pid))
                            .cloned()
                            .unwrap_or_default();
                        (*pid, locs)
                    })
                    .collect()
            })
            .unwrap_or_default();

        (self.active_session_id, snapshot)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap, HashSet};

    use multiaddr::multiaddr;
    use nomos_core::sdp::{
        FinalizedBlockEvent, FinalizedBlockEventUpdate, FinalizedDeclarationState, Locator,
        ProviderId, ServiceType,
    };

    use super::{MembershipBackend as _, MockMembershipBackend, MockMembershipBackendSettings};

    fn pid(seed: u8) -> ProviderId {
        let mut b = [0u8; 32];
        b[0] = seed;
        ProviderId(b)
    }

    fn locator(seed: u8) -> Locator {
        Locator::new(multiaddr!(
            Ip4([10, 0, 0, seed]),
            Udp(8000u16 + u16::from(seed))
        ))
    }

    fn locs<const N: usize>(base: u8) -> BTreeSet<Locator> {
        (0..N).map(|i| locator(base + i as u8)).collect()
    }

    fn update(
        service: ServiceType,
        provider_id: ProviderId,
        state: FinalizedDeclarationState,
        locators: BTreeSet<Locator>,
    ) -> FinalizedBlockEventUpdate {
        FinalizedBlockEventUpdate {
            service_type: service,
            provider_id,
            state,
            locators,
        }
    }

    #[tokio::test]
    async fn init_returns_seeded_session_zero() {
        let service = ServiceType::DataAvailability;

        // Seed session 0 with P1 and its locators
        let p1 = pid(1);
        let p1_locs = locs::<2>(1);

        let mut session0_membership: HashMap<ServiceType, HashSet<ProviderId>> = HashMap::new();
        session0_membership.insert(service, HashSet::from([p1]));

        let mut session0_locators = HashMap::new();
        session0_locators.insert(service, HashMap::new());
        session0_locators
            .get_mut(&service)
            .unwrap()
            .insert(p1, p1_locs.clone());

        let settings = MockMembershipBackendSettings {
            session_size_blocks: 3,
            session_zero_membership: session0_membership,
            session_zero_locators_mapping: session0_locators,
        };

        let backend = MockMembershipBackend::init(settings);

        // Active snapshot is seeded session 0
        let (sid, providers) = backend.get_latest_providers(service).await.unwrap();
        assert_eq!(sid, 0);
        assert_eq!(providers.len(), 1);
        assert_eq!(providers.get(&p1).unwrap(), &p1_locs);
    }

    #[tokio::test]
    async fn forming_promotes_on_last_block_of_session() {
        let service = ServiceType::DataAvailability;

        // Session 0: P1 active
        let p1 = pid(1);
        let p1_locs = locs::<2>(1);

        let mut session0_membership: HashMap<ServiceType, HashSet<ProviderId>> = HashMap::new();
        session0_membership.insert(service, HashSet::from([p1]));

        let mut session0_locators = HashMap::new();
        session0_locators.insert(service, HashMap::new());
        session0_locators
            .get_mut(&service)
            .unwrap()
            .insert(p1, p1_locs.clone());

        // Small session size for easy boundary testing
        let settings = MockMembershipBackendSettings {
            session_size_blocks: 3, // blocks 0,1,2 => session 0; 3,4,5 => session 1; etc.
            session_zero_membership: session0_membership,
            session_zero_locators_mapping: session0_locators,
        };

        let mut backend = MockMembershipBackend::init(settings);

        // Forming session 1 updates across blocks 1..2 (still session 0 time)
        let p2 = pid(2);
        let p2_locs = locs::<3>(10);

        // Block 1: activate P2 (goes into forming S=1)
        let ev1 = FinalizedBlockEvent {
            block_number: 1,
            updates: vec![update(
                service,
                p2,
                FinalizedDeclarationState::Active,
                p2_locs.clone(),
            )],
        };
        let r1 = backend.update(ev1).await.unwrap();
        assert!(r1.is_none(), "No promotion before session boundary");

        // Active snapshot still session 0 (only P1)
        let (sid_a1, prov_a1) = backend.get_latest_providers(service).await.unwrap();
        assert_eq!(sid_a1, 0);
        assert_eq!(prov_a1.len(), 1);
        assert!(prov_a1.contains_key(&p1));

        // Block 2 (last block of session 0): withdraw P1 in forming; still in active
        // until promotion
        let ev2 = FinalizedBlockEvent {
            block_number: 2,
            updates: vec![update(
                service,
                p1,
                FinalizedDeclarationState::Withdrawn,
                BTreeSet::new(),
            )],
        };

        // For block=2, block+1=3 -> 3/3==1 => forming_session_id==1 -> promote now.
        let r2 = backend.update(ev2).await.unwrap();
        let promoted = r2.expect("Promotion should occur at the end of session 0");

        // The returned snapshot should be the new active (session 1) view
        let (sid_promoted, prov_promoted) = promoted.get(&service).unwrap();
        assert_eq!(*sid_promoted, 1);
        assert_eq!(prov_promoted.len(), 1);
        assert_eq!(prov_promoted.get(&p2).unwrap(), &p2_locs);

        // And get_latest_providers must match the new active snapshot
        let (sid_a2, prov_a2) = backend.get_latest_providers(service).await.unwrap();
        assert_eq!(sid_a2, 1);
        assert_eq!(prov_a2, *prov_promoted);
    }
}
