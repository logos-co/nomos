use std::collections::{HashMap, HashSet};

use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{FinalizedDeclarationState, ProviderId, ServiceType},
};
use strum::IntoEnumIterator;

pub struct SessionConfig {
    size: BlockNumber,
}

#[derive(Clone)]
pub struct Session {
    pub session_number: SessionNumber,
    pub providers: HashSet<ProviderId>,
}

impl Session {
    pub fn update(&mut self, provider_id: ProviderId, state: FinalizedDeclarationState) {
        match state {
            FinalizedDeclarationState::Active => {
                self.providers.insert(provider_id);
            }
            FinalizedDeclarationState::Inactive | FinalizedDeclarationState::Withdrawn => {
                self.providers.remove(&provider_id);
            }
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum MembershipError {
    #[error("Active session for service {0:?} not found")]
    ActiveSessionNotFound(ServiceType),
    #[error("Forming session for service {0:?} not found")]
    FormingSessionNotFound(ServiceType),
}

pub struct Membership {
    active_sessions: rpds::HashTrieMapSync<ServiceType, Session>,
    forming_sessions: rpds::HashTrieMapSync<ServiceType, Session>,
}

impl Membership {
    #[must_use]
    pub fn new() -> Self {
        let mut active_sessions = rpds::HashTrieMapSync::new_sync();
        let mut forming_sessions = rpds::HashTrieMapSync::new_sync();

        for service_type in ServiceType::iter() {
            let initial_active_session = Session {
                session_number: 0,
                providers: HashSet::new(),
            };
            let initial_forming_session = Session {
                session_number: 1,
                providers: HashSet::new(),
            };

            active_sessions = active_sessions.insert(service_type, initial_active_session);
            forming_sessions = forming_sessions.insert(service_type, initial_forming_session);
        }

        Self {
            active_sessions,
            forming_sessions,
        }
    }

    pub fn update(
        mut self,
        service_type: &ServiceType,
        provider_id: ProviderId,
        state: FinalizedDeclarationState,
    ) -> Result<Self, MembershipError> {
        let Some(forming_session) = self.forming_sessions.get_mut(service_type) else {
            return Err(MembershipError::FormingSessionNotFound(*service_type));
        };
        forming_session.update(provider_id, state);

        Ok(self)
    }

    pub fn update_from_genesis(
        mut self,
        service_type: &ServiceType,
        provider_id: ProviderId,
        state: FinalizedDeclarationState,
    ) -> Result<Self, MembershipError> {
        let Some(active_session) = self.active_sessions.get_mut(service_type) else {
            return Err(MembershipError::ActiveSessionNotFound(*service_type));
        };
        active_session.update(provider_id, state);

        Ok(self)
    }

    pub fn promote(
        mut self,
        block_number: BlockNumber,
        session_configs: &HashMap<ServiceType, SessionConfig>,
    ) -> Result<Self, MembershipError> {
        let mut new_active_sessions = self.active_sessions;
        let mut new_forming_sessions = self.forming_sessions;

        for service_type in ServiceType::iter() {
            let Some(config) = session_configs.get(&service_type) else {
                continue;
            };

            let Some(forming_session) = new_forming_sessions.get(&service_type) else {
                continue;
            };

            let expected_active_session_num = block_number / config.size;
            if forming_session.session_number > expected_active_session_num {
                continue;
            }

            let new_active_session = forming_session.clone();

            let mut next_forming_session = new_active_session.clone();
            next_forming_session.session_number += 1;

            new_active_sessions = new_active_sessions.insert(service_type, new_active_session);
            new_forming_sessions = new_forming_sessions.insert(service_type, next_forming_session);
        }

        self.active_sessions = new_active_sessions;
        self.forming_sessions = new_forming_sessions;

        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl SessionConfig {
        fn new(size: BlockNumber) -> Self {
            Self { size }
        }
    }

    fn setup() -> (Membership, HashMap<ServiceType, SessionConfig>) {
        let mut configs = HashMap::new();
        configs.insert(ServiceType::BlendNetwork, SessionConfig::new(10));
        configs.insert(ServiceType::DataAvailability, SessionConfig::new(5));

        let membership = Membership::new();

        (membership, configs)
    }

    #[test]
    fn test_add_provider_forming_session() {
        let (membership, _) = setup();
        let service_a: ServiceType = ServiceType::BlendNetwork;
        let provider_id = ProviderId::try_from([1; 32]).unwrap();

        let updated_membership = membership
            .update(&service_a, provider_id, FinalizedDeclarationState::Active)
            .unwrap();

        let forming_session = updated_membership.forming_sessions.get(&service_a).unwrap();
        assert!(forming_session.providers.contains(&provider_id));
        assert_eq!(forming_session.providers.len(), 1);
    }

    #[test]
    fn test_remove_provider_forming_session() {
        let (membership, _) = setup();
        let service_a: ServiceType = ServiceType::BlendNetwork;
        let provider_id = ProviderId::try_from([1; 32]).unwrap();

        let with_provider = membership
            .update(&service_a, provider_id, FinalizedDeclarationState::Active)
            .unwrap();

        let without_provider = with_provider
            .update(&service_a, provider_id, FinalizedDeclarationState::Inactive)
            .unwrap();

        let forming_session = without_provider.forming_sessions.get(&service_a).unwrap();
        assert!(!forming_session.providers.contains(&provider_id));
        assert!(forming_session.providers.is_empty());
    }

    #[test]
    fn test_promote_session() {
        let (membership, configs) = setup();
        let service_a: ServiceType = ServiceType::BlendNetwork;
        let provider_id = ProviderId::try_from([1; 32]).unwrap();

        let updated_membership = membership
            .update(&service_a, provider_id, FinalizedDeclarationState::Active)
            .unwrap();

        let promoted_membership = updated_membership.promote(10, &configs).unwrap();

        let active_session = promoted_membership.active_sessions.get(&service_a).unwrap();
        assert_eq!(active_session.session_number, 1);
        assert!(active_session.providers.contains(&provider_id));

        let forming_session = promoted_membership
            .forming_sessions
            .get(&service_a)
            .unwrap();
        assert_eq!(forming_session.session_number, 2);
        assert!(forming_session.providers.contains(&provider_id));
    }

    #[test]
    fn test_no_promotion() {
        let (membership, configs) = setup();
        let service_a: ServiceType = ServiceType::BlendNetwork;

        let promoted_membership = membership.promote(9, &configs).unwrap();

        let active_session = promoted_membership.active_sessions.get(&service_a).unwrap();
        assert_eq!(active_session.session_number, 0);
        assert!(active_session.providers.is_empty());

        let forming_session = promoted_membership
            .forming_sessions
            .get(&service_a)
            .unwrap();
        assert_eq!(forming_session.session_number, 1);
    }

    #[test]
    fn test_promote_due_service() {
        let (membership, configs) = setup();
        let service_a: ServiceType = ServiceType::BlendNetwork;
        let service_b: ServiceType = ServiceType::DataAvailability;

        let promoted_membership = membership.promote(6, &configs).unwrap();
        assert_eq!(
            promoted_membership
                .active_sessions
                .get(&service_b)
                .unwrap()
                .session_number,
            1
        );
        assert_eq!(
            promoted_membership
                .forming_sessions
                .get(&service_b)
                .unwrap()
                .session_number,
            2
        );

        let active_session_a = promoted_membership.active_sessions.get(&service_a).unwrap();
        assert_eq!(active_session_a.session_number, 0);

        let forming_session_a = promoted_membership
            .forming_sessions
            .get(&service_a)
            .unwrap();
        assert_eq!(forming_session_a.session_number, 1);
    }
}
