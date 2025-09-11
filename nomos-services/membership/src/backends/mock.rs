use std::{
    collections::{BTreeSet, HashMap},
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
    providers: HashMap<ProviderId, BTreeSet<Locator>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockMembershipBackendSettings {
    pub session_sizes: HashMap<ServiceType, u32>,
    // Initial state for each service type - combines membership and locators
    pub session_zero_providers: HashMap<ServiceType, HashMap<ProviderId, BTreeSet<Locator>>>,
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

        for service_type in settings.session_sizes.keys() {
            let providers = settings
                .session_zero_providers
                .get(service_type)
                .cloned()
                .unwrap_or_default();

            let session_0 = SessionState {
                session_number: 0,
                providers,
            };

            active_sessions.insert(*service_type, session_0.clone());

            let mut session_1 = session_0;
            session_1.session_number = 1;
            forming_sessions.insert(*service_type, session_1);
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
                self.providers
                    .insert(update.provider_id, update.locators.clone());
            }
            nomos_core::sdp::FinalizedDeclarationState::Inactive
            | nomos_core::sdp::FinalizedDeclarationState::Withdrawn => {
                self.providers.remove(&update.provider_id);
            }
        }
    }

    fn to_snapshot(&self) -> MembershipProviders {
        let providers = self.providers.clone();
        (self.session_number, providers)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use multiaddr::multiaddr;
    use nomos_core::sdp::{
        FinalizedBlockEvent, FinalizedBlockEventUpdate, FinalizedDeclarationState, Locator,
        ProviderId, ServiceType,
    };

    use super::{MembershipBackend as _, MockMembershipBackend, MockMembershipBackendSettings};

    fn pid(seed: u8) -> ProviderId {
        use ed25519_dalek::SigningKey;

        // Create a deterministic signing key from seed
        let secret_bytes = [seed; 32];
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();

        ProviderId(verifying_key)
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

        let mut session0_providers = HashMap::new();
        session0_providers.insert(service, HashMap::from([(p1, p1_locs.clone())]));

        let settings = MockMembershipBackendSettings {
            session_sizes: HashMap::from([(service, 3)]),
            session_zero_providers: session0_providers,
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

        let mut session0_providers = HashMap::new();
        session0_providers.insert(service, HashMap::from([(p1, p1_locs.clone())]));

        let settings = MockMembershipBackendSettings {
            session_sizes: HashMap::from([(service, 3)]),
            session_zero_providers: session0_providers,
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

    #[tokio::test]
    async fn multiple_service_types_with_different_session_sizes() {
        let service_da = ServiceType::DataAvailability;
        let service_mp = ServiceType::BlendNetwork;

        // Set up initial providers
        let p1 = pid(1);
        let p1_locs = locs::<2>(1);
        let p2 = pid(2);
        let p2_locs = locs::<2>(10);

        // Session 0 providers: P1 in DA, P2 in BlendNetwork
        let mut session0_providers = HashMap::new();
        session0_providers.insert(service_da, HashMap::from([(p1, p1_locs.clone())]));
        session0_providers.insert(service_mp, HashMap::from([(p2, p2_locs.clone())]));

        // Different session sizes: DA=3 blocks, BlendNetwork=5 blocks
        let settings = MockMembershipBackendSettings {
            session_sizes: HashMap::from([
                (service_da, 3), // DA sessions: 0-2, 3-5, 6-8...
                (service_mp, 5), // MP sessions: 0-4, 5-9, 10-14...
            ]),
            session_zero_providers: session0_providers,
        };
        let mut backend = MockMembershipBackend::init(settings);

        // Add new providers to forming sessions
        let p3 = pid(3);
        let p3_locs = locs::<2>(20);
        let p4 = pid(4);
        let p4_locs = locs::<2>(30);

        // Block 1: Add P3 to DA, P4 to Blend Network (both in forming)
        let ev1 = FinalizedBlockEvent {
            block_number: 1,
            updates: vec![
                update(
                    service_da,
                    p3,
                    FinalizedDeclarationState::Active,
                    p3_locs.clone(),
                ),
                update(
                    service_mp,
                    p4,
                    FinalizedDeclarationState::Active,
                    p4_locs.clone(),
                ),
            ],
        };
        backend.update(ev1).await.unwrap();

        // Block 2: DA should promote after this (end of session 0), Blend Network
        // should not
        let ev2 = FinalizedBlockEvent {
            block_number: 2,
            updates: vec![],
        };
        let result2 = backend.update(ev2).await.unwrap();

        // Only DA should have promoted
        let promoted = result2.expect("DA should promote at block 2");
        assert_eq!(promoted.len(), 1);
        assert!(promoted.contains_key(&service_da));
        assert!(!promoted.contains_key(&service_mp));

        // DA should now be in session 1 with P3
        let (da_sid, da_providers) = promoted.get(&service_da).unwrap();
        assert_eq!(*da_sid, 1);
        assert!(da_providers.contains_key(&p3));
        assert!(da_providers.contains_key(&p1)); // P1 is not withdrawn

        // Blend Network should still be in session 0
        let (mp_sid, mp_providers) = backend.get_latest_providers(service_mp).await.unwrap();
        assert_eq!(mp_sid, 0);
        assert!(mp_providers.contains_key(&p2)); // Still has original
        assert!(!mp_providers.contains_key(&p4)); // P4 still in forming

        // Block 4: Blend Network should promote after this (end of session 0)
        let ev3 = FinalizedBlockEvent {
            block_number: 4,
            updates: vec![],
        };
        let result3 = backend.update(ev3).await.unwrap();

        // Only Blend Network should promote
        let promoted = result3.expect("Blend Network should promote at block 4");
        assert_eq!(promoted.len(), 1);
        assert!(promoted.contains_key(&service_mp));

        // Blend Network should now be in session 1 with P4
        let (mp_sid, mp_providers) = promoted.get(&service_mp).unwrap();
        assert_eq!(*mp_sid, 1);
        assert!(mp_providers.contains_key(&p4));
        assert!(mp_providers.contains_key(&p2)); // P2 still there
    }
}
