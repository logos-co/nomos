use std::collections::HashMap;

use nomos_core::{
    block::BlockNumber,
    sdp::{FinalizedBlockEvent, ServiceType},
};

use crate::{
    MembershipConfig, MembershipError, NewSesssion, session::Session, storage::MembershipStorage,
};

pub struct Membership<S: MembershipStorage> {
    storage: S,
    active_sessions: HashMap<ServiceType, Session>,
    forming_sessions: HashMap<ServiceType, Session>,
    latest_block_number: BlockNumber,
    session_sizes: HashMap<ServiceType, u32>,
}

impl<S> Membership<S>
where
    S: MembershipStorage,
{
    pub fn new(settings: MembershipConfig, storage_adapter: S) -> Self {
        let mut active_sessions = HashMap::new();
        let mut forming_sessions = HashMap::new();

        for service_type in settings.session_sizes.keys() {
            let providers = settings
                .session_zero_providers
                .get(service_type)
                .cloned()
                .unwrap_or_default();

            let session_0 = Session {
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
            storage: storage_adapter,
        }
    }

    pub async fn update(
        &mut self,
        update: FinalizedBlockEvent,
    ) -> Result<NewSesssion, MembershipError> {
        let block_number = update.block_number;

        tracing::debug!(
            "Updating membership: {:?}, block {}, latest_block_number: {}",
            update,
            block_number,
            self.latest_block_number
        );

        if block_number <= self.latest_block_number {
            return Err(MembershipError::BlockFromPast);
        }

        // Apply updates to forming sessions
        for event_update in update.updates {
            if let Some(forming_session) = self.forming_sessions.get_mut(&event_update.service_type)
            {
                forming_session.apply_update(&event_update);
            }
        }

        self.latest_block_number = block_number;

        // Persist latest block
        self.storage
            .save_latest_block(block_number)
            .await
            .map_err(MembershipError::Other)?;

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
                    completed_sessions.insert(*service_type, promoted_session.clone());

                    // Persist the new active session
                    self.storage
                        .save_active_session(
                            *service_type,
                            promoted_session.session_number,
                            &promoted_session.providers,
                        )
                        .await
                        .map_err(MembershipError::Other)?;

                    // Promote forming to active
                    self.active_sessions
                        .insert(*service_type, promoted_session.clone());

                    // Start new forming session with incremented session number
                    let mut new_forming = promoted_session;
                    new_forming.session_number = next_session + 1;

                    // Persist new forming session
                    self.storage
                        .save_forming_session(
                            *service_type,
                            new_forming.session_number,
                            &new_forming.providers,
                        )
                        .await
                        .map_err(MembershipError::Other)?;

                    self.forming_sessions.insert(*service_type, new_forming);
                } else {
                    // Just persist the updated forming session
                    self.storage
                        .save_forming_session(
                            *service_type,
                            forming_session.session_number,
                            &forming_session.providers,
                        )
                        .await
                        .map_err(MembershipError::Other)?;
                }
            }
        }

        Ok(if completed_sessions.is_empty() {
            None
        } else {
            Some(completed_sessions)
        })
    }

    pub fn get_latest_providers(
        &self,
        service_type: ServiceType,
    ) -> Result<Session, MembershipError> {
        if let Some(session_state) = self.active_sessions.get(&service_type).cloned() {
            return Ok(session_state);
        }

        Err(MembershipError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap},
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use multiaddr::multiaddr;
    use nomos_core::{
        block::{BlockNumber, SessionNumber},
        sdp::{
            FinalizedBlockEvent, FinalizedBlockEventUpdate, FinalizedDeclarationState, Locator,
            ProviderId, ServiceType,
        },
    };

    use crate::{
        DynError,
        membership::{Membership, MembershipConfig},
    };

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
        let storage = InMemoryStorageArc::new();

        let service = ServiceType::DataAvailability;

        // Seed session 0 with P1 and its locators
        let p1 = pid(1);
        let p1_locs = locs::<2>(1);

        let mut session0_providers = HashMap::new();
        session0_providers.insert(service, HashMap::from([(p1, p1_locs.clone())]));

        let settings = MembershipConfig {
            session_sizes: HashMap::from([(service, 3)]),
            session_zero_providers: session0_providers,
        };

        let pmembership = Membership::new(settings, storage);

        // Active snapshot is seeded session 0
        let session_state = pmembership.get_latest_providers(service).unwrap();
        assert_eq!(session_state.session_number, 0);
        assert_eq!(session_state.providers.len(), 1);
        assert_eq!(session_state.providers.get(&p1).unwrap(), &p1_locs);
    }

    #[tokio::test]
    async fn forming_promotes_on_last_block_of_session() {
        let storage = InMemoryStorageArc::new();
        let service = ServiceType::DataAvailability;

        // Session 0: P1 active
        let p1 = pid(1);
        let p1_locs = locs::<2>(1);

        let mut session0_providers = HashMap::new();
        session0_providers.insert(service, HashMap::from([(p1, p1_locs.clone())]));

        let settings = MembershipConfig {
            session_sizes: HashMap::from([(service, 3)]),
            session_zero_providers: session0_providers,
        };
        let mut pmembership = Membership::new(settings, storage);

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
        let r1 = pmembership.update(ev1).await.unwrap();
        assert!(r1.is_none(), "No promotion before session boundary");

        // Active snapshot still session 0 (only P1)
        let session_state_1 = pmembership.get_latest_providers(service).unwrap();
        assert_eq!(session_state_1.session_number, 0);
        assert_eq!(session_state_1.providers.len(), 1);
        assert!(session_state_1.providers.contains_key(&p1));

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
        let r2 = pmembership.update(ev2).await.unwrap();
        let promoted = r2.expect("Promotion should occur at the end of session 0");

        // The returned snapshot should be the new active (session 1) view
        let s_promoted = promoted.get(&service).unwrap();
        assert_eq!(s_promoted.session_number, 1);
        assert_eq!(s_promoted.providers.len(), 1);
        assert_eq!(s_promoted.providers.get(&p2).unwrap(), &p2_locs);

        // And get_latest_providers must match the new active snapshot
        let s_2 = pmembership.get_latest_providers(service).unwrap();
        assert_eq!(s_2.session_number, 1);
        assert_eq!(s_2.providers, s_promoted.providers);
    }

    #[tokio::test]
    async fn multiple_service_types_with_different_session_sizes() {
        let storage = InMemoryStorageArc::new();
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
        let settings = MembershipConfig {
            session_sizes: HashMap::from([
                (service_da, 3), // DA sessions: 0-2, 3-5, 6-8...
                (service_mp, 5), // MP sessions: 0-4, 5-9, 10-14...
            ]),
            session_zero_providers: session0_providers,
        };
        let mut pmembership = Membership::new(settings, storage);

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
        pmembership.update(ev1).await.unwrap();

        // Block 2: DA should promote after this (end of session 0), Blend Network
        // should not
        let ev2 = FinalizedBlockEvent {
            block_number: 2,
            updates: vec![],
        };
        let result2 = pmembership.update(ev2).await.unwrap();

        // Only DA should have promoted
        let promoted = result2.expect("DA should promote at block 2");
        assert_eq!(promoted.len(), 1);
        assert!(promoted.contains_key(&service_da));
        assert!(!promoted.contains_key(&service_mp));

        // DA should now be in session 1 with P3
        let da_session_state = promoted.get(&service_da).unwrap();
        assert_eq!(da_session_state.session_number, 1);
        assert!(da_session_state.providers.contains_key(&p3));
        assert!(da_session_state.providers.contains_key(&p1)); // P1 is not withdrawn

        // Blend Network should still be in session 0
        let mp_ss = pmembership.get_latest_providers(service_mp).unwrap();
        assert_eq!(mp_ss.session_number, 0);
        assert!(mp_ss.providers.contains_key(&p2)); // Still has original
        assert!(!mp_ss.providers.contains_key(&p4)); // P4 still in forming

        // Block 4: Blend Network should promote after this (end of session 0)
        let ev3 = FinalizedBlockEvent {
            block_number: 4,
            updates: vec![],
        };
        let result3 = pmembership.update(ev3).await.unwrap();

        // Only Blend Network should promote
        let promoted = result3.expect("Blend Network should promote at block 4");
        assert_eq!(promoted.len(), 1);
        assert!(promoted.contains_key(&service_mp));

        // Blend Network should now be in session 1 with P4
        let mp_ss = promoted.get(&service_mp).unwrap();
        assert_eq!(mp_ss.session_number, 1);
        assert!(mp_ss.providers.contains_key(&p4));
        assert!(mp_ss.providers.contains_key(&p2)); // P2 still there
    }

    #[derive(Clone)]
    pub struct InMemoryStorageArc {
        data: Arc<Mutex<InMemoryStorage>>,
    }

    #[derive(Default)]
    struct InMemoryStorage {
        active_sessions:
            HashMap<ServiceType, (SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>,
        forming_sessions:
            HashMap<ServiceType, (SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>,
        latest_block: Option<BlockNumber>,
    }

    impl InMemoryStorageArc {
        #[must_use]
        pub fn new() -> Self {
            Self {
                data: Arc::new(Mutex::new(InMemoryStorage::default())),
            }
        }
    }

    #[async_trait]
    impl super::MembershipStorage for InMemoryStorageArc {
        async fn save_active_session(
            &mut self,
            service_type: ServiceType,
            session_id: SessionNumber,
            providers: &HashMap<ProviderId, BTreeSet<Locator>>,
        ) -> Result<(), DynError> {
            self.data
                .lock()
                .unwrap()
                .active_sessions
                .insert(service_type, (session_id, providers.clone()));
            Ok(())
        }

        async fn load_active_session(
            &mut self,
            service_type: ServiceType,
        ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, DynError>
        {
            Ok(self
                .data
                .lock()
                .unwrap()
                .active_sessions
                .get(&service_type)
                .cloned())
        }

        async fn save_latest_block(&mut self, block_number: BlockNumber) -> Result<(), DynError> {
            self.data.lock().unwrap().latest_block = Some(block_number);
            Ok(())
        }

        async fn load_latest_block(&mut self) -> Result<Option<BlockNumber>, DynError> {
            Ok(self.data.lock().unwrap().latest_block)
        }

        async fn save_forming_session(
            &mut self,
            service_type: ServiceType,
            session_id: SessionNumber,
            providers: &HashMap<ProviderId, BTreeSet<Locator>>,
        ) -> Result<(), DynError> {
            self.data
                .lock()
                .unwrap()
                .forming_sessions
                .insert(service_type, (session_id, providers.clone()));
            Ok(())
        }

        async fn load_forming_session(
            &mut self,
            service_type: ServiceType,
        ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, DynError>
        {
            Ok(self
                .data
                .lock()
                .unwrap()
                .forming_sessions
                .get(&service_type)
                .cloned())
        }
    }
}
