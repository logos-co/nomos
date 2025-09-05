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

#[derive(Debug, Clone)]
struct SessionState {
    session_number: SessionNumber,
    membership: HashSet<ProviderId>,
    locators: HashMap<ProviderId, BTreeSet<Locator>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockMembershipBackendSettings {
    pub session_sizes: HashMap<ServiceType, u32>,
    // Initial state for each service type
    pub session_zero_memberships: HashMap<ServiceType, HashSet<ProviderId>>,
    pub session_zero_locators: HashMap<ServiceType, HashMap<ProviderId, BTreeSet<Locator>>>,
}

pub struct MockMembershipBackend {
    active_sessions: HashMap<ServiceType, SessionState>,
    forming_sessions: HashMap<ServiceType, SessionState>,
    latest_block_number: BlockNumber,
    session_sizes: HashMap<ServiceType, u32>,
}

#[async_trait::async_trait]
impl MembershipBackend for MockMembershipBackend {
    type Settings = MockMembershipBackendSettings;

    fn init(settings: MockMembershipBackendSettings) -> Self {
        let mut active_sessions = HashMap::new();
        let mut forming_sessions = HashMap::new();

        for (service_type, members) in settings.session_zero_memberships {
            let locators = settings
                .session_zero_locators
                .get(&service_type)
                .cloned()
                .unwrap_or_default();

            let session_0 = SessionState {
                session_number: 0,
                membership: members.clone(),
                locators: locators.clone(),
            };

            active_sessions.insert(service_type, session_0.clone());

            // Forming starts at session 1
            let mut session_1 = session_0;
            session_1.session_number = 1;
            forming_sessions.insert(service_type, session_1);
        }

        Self {
            active_sessions,
            forming_sessions,
            latest_block_number: 0,
            session_sizes: settings.session_sizes,
        }
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

        // Apply updates to forming sessions of relevant service types
        for event_update in update.updates {
            if let Some(forming_session) = self.forming_sessions.get_mut(&event_update.service_type)
            {
                forming_session.apply_update(&event_update);
            }
        }

        self.latest_block_number = block_number;

        // Check which services complete their sessions at this block
        let mut completed_sessions = HashMap::new();

        for (service_type, session_size) in &self.session_sizes {
            let next_session = (block_number + 1) / BlockNumber::from(*session_size);

            if let Some(forming_session) = self.forming_sessions.get(service_type) {
                // Check if forming session should be promoted
                if forming_session.session_number <= next_session {
                    // Clone forming session to promote it
                    let promoted_session = forming_session.clone();

                    // Get snapshot before promoting
                    completed_sessions.insert(*service_type, promoted_session.to_snapshot());

                    // Promote forming to active
                    self.active_sessions
                        .insert(*service_type, promoted_session.clone());

                    // Start new forming session with incremented session number
                    let mut new_forming = promoted_session;
                    new_forming.session_number = next_session + 1;
                    self.forming_sessions.insert(*service_type, new_forming);
                }
            }
        }

        Ok(if completed_sessions.is_empty() {
            None
        } else {
            Some(completed_sessions)
        })
    }

    async fn get_latest_providers(
        &self,
        service_type: ServiceType,
    ) -> Result<MembershipProviders, MembershipBackendError> {
        Ok(self
            .active_sessions
            .get(&service_type)
            .map_or_else(|| (0, HashMap::new()), SessionState::to_snapshot))
    }
}

impl SessionState {
    fn apply_update(&mut self, update: &FinalizedBlockEventUpdate) {
        match update.state {
            nomos_core::sdp::FinalizedDeclarationState::Active => {
                self.membership.insert(update.provider_id);
                self.locators
                    .insert(update.provider_id, update.locators.clone());
            }
            nomos_core::sdp::FinalizedDeclarationState::Inactive
            | nomos_core::sdp::FinalizedDeclarationState::Withdrawn => {
                self.membership.remove(&update.provider_id);
                self.locators.remove(&update.provider_id);
            }
        }
    }

    fn to_snapshot(&self) -> MembershipProviders {
        let providers = self
            .membership
            .iter()
            .map(|pid| (*pid, self.locators.get(pid).cloned().unwrap_or_default()))
            .collect();
        (self.session_number, providers)
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
            session_sizes: HashMap::from([(service, 3)]),
            session_zero_memberships: session0_membership,
            session_zero_locators: session0_locators,
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
            session_sizes: HashMap::from([(service, 3)]),
            session_zero_memberships: session0_membership,
            session_zero_locators: session0_locators,
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
