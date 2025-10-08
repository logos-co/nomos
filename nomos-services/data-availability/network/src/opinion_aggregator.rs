use std::collections::{BTreeMap, HashMap, HashSet};

use bitvec::prelude::*;
use libp2p::PeerId;
use nomos_core::{block::SessionNumber, sdp::ProviderId};
use nomos_da_network_core::{
    SubnetworkId,
    protocols::sampling::opinions::{Opinion, OpinionEvent},
};
use overwatch::DynError;
use thiserror::Error;

use crate::{
    membership::{Assignations, SubnetworkPeers},
    storage::MembershipStorageAdapter,
};

const OPINION_THRESHOLD: u32 = 10;

#[derive(Error, Debug)]
pub enum OpinionError {
    /// First session change on startup has both current and previous membership
    /// set to 0
    #[error("Insufficient data to generate opinions (session 0 or initial startup)")]
    InsufficientData,

    #[error("Storage error: {0}")]
    Error(#[from] DynError),
}

pub struct Opinions {
    pub session_id: SessionNumber,
    pub new_opinions: BitVec,
    pub old_opinions: BitVec,
}

struct Membership {
    session_id: SessionNumber,
    peers: HashSet<PeerId>,
    provider_mappings: HashMap<PeerId, ProviderId>,
}

impl Membership {
    fn empty(session_id: SessionNumber) -> Self {
        Self {
            session_id,
            peers: HashSet::new(),
            provider_mappings: HashMap::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    fn members(&self) -> Vec<PeerId> {
        self.peers.iter().copied().collect()
    }
}

impl From<&SubnetworkPeers<PeerId>> for Membership {
    fn from(subnet_peers: &SubnetworkPeers<PeerId>) -> Self {
        Self {
            session_id: subnet_peers.session_id,
            peers: subnet_peers.peers.keys().copied().collect(),
            provider_mappings: subnet_peers.provider_mappings.clone(),
        }
    }
}

pub struct OpinionAggregator<Storage> {
    storage: Storage,

    local_peer_id: PeerId,
    local_provider_id: ProviderId,

    positive_opinions: HashMap<PeerId, u32>,
    negative_opinions: HashMap<PeerId, u32>,
    blacklist: HashSet<PeerId>,

    prev_session_positive_opinions: HashMap<PeerId, u32>,
    prev_session_negative_opinions: HashMap<PeerId, u32>,
    old_blacklist: HashSet<PeerId>,

    current_membership: Option<Membership>,
    previous_membership: Option<Membership>,
}

impl<Storage> OpinionAggregator<Storage>
where
    Storage: MembershipStorageAdapter<PeerId, SubnetworkId> + Send + Sync,
{
    pub fn new(storage: Storage, local_peer_id: PeerId, local_provider_id: ProviderId) -> Self {
        Self {
            storage,
            local_peer_id,
            local_provider_id,
            positive_opinions: HashMap::new(),
            negative_opinions: HashMap::new(),
            blacklist: HashSet::new(),
            prev_session_positive_opinions: HashMap::new(),
            prev_session_negative_opinions: HashMap::new(),
            old_blacklist: HashSet::new(),
            current_membership: None,
            previous_membership: None,
        }
    }

    pub fn record_opinion(&mut self, event: OpinionEvent) {
        for opinion in event.opinions {
            match opinion {
                Opinion::Positive {
                    peer_id,
                    session_id,
                } => {
                    self.handle_positive(peer_id, session_id);
                }
                Opinion::Negative {
                    peer_id,
                    session_id,
                } => {
                    self.handle_negative(peer_id, session_id);
                }
                Opinion::Blacklist {
                    peer_id,
                    session_id,
                } => {
                    self.handle_blacklist(peer_id, session_id);
                }
            }
        }
    }

    fn handle_positive(&mut self, peer_id: PeerId, session_id: SessionNumber) {
        if let Some(ref current) = self.current_membership
            && current.session_id == session_id
        {
            if let Some(count) = self.positive_opinions.get_mut(&peer_id) {
                *count += 1;
            }
            return;
        }
        if let Some(ref previous) = self.previous_membership
            && previous.session_id == session_id
            && let Some(count) = self.prev_session_positive_opinions.get_mut(&peer_id)
        {
            *count += 1;
        }
    }

    fn handle_negative(&mut self, peer_id: PeerId, session_id: SessionNumber) {
        if let Some(ref current) = self.current_membership
            && current.session_id == session_id
        {
            if let Some(count) = self.negative_opinions.get_mut(&peer_id) {
                *count += 1;
            }
            return;
        }
        if let Some(ref previous) = self.previous_membership
            && previous.session_id == session_id
            && let Some(count) = self.prev_session_negative_opinions.get_mut(&peer_id)
        {
            *count += 1;
        }
    }

    fn handle_blacklist(&mut self, peer_id: PeerId, session_id: SessionNumber) {
        if let Some(ref current) = self.current_membership
            && current.session_id == session_id
        {
            if let Some(count) = self.positive_opinions.get_mut(&peer_id) {
                *count = 0;
            }
            self.blacklist.insert(peer_id);
            return;
        }
        if let Some(ref previous) = self.previous_membership
            && previous.session_id == session_id
        {
            if let Some(count) = self.prev_session_positive_opinions.get_mut(&peer_id) {
                *count = 0;
            }
            self.old_blacklist.insert(peer_id);
        }
    }

    pub async fn handle_session_change(
        &mut self,
        new_membership: &SubnetworkPeers<PeerId>,
    ) -> Result<Opinions, OpinionError> {
        let new_session_id = new_membership.session_id;

        // Case 1: Service is starting (current membership is None)
        if self.current_membership.is_none() {
            // Case 1.1: Session 0 (genesis)
            if new_session_id == 0 {
                self.current_membership = Some(Membership::from(new_membership));
                self.reset_opinion_maps();
                return Err(OpinionError::InsufficientData);
            }

            // Case 1.2: Regular restart (session > 0)
            // Restore current membership (session we're generating opinions for)
            let current_session_id = new_session_id - 1;

            // Get assignations and convert to Membership
            match self.storage.get(current_session_id).await {
                Ok(Some(assignations)) => {
                    let membership = self
                        .assignations_to_membership(assignations, current_session_id)
                        .await?;
                    self.current_membership = Some(membership);
                }
                Ok(None) => {
                    // Not found - set empty membership
                    self.current_membership = Some(Membership::empty(current_session_id));
                }
                Err(e) => return Err(OpinionError::Error(e)),
            }

            // Restore previous membership (if current_session_id > 0)
            if current_session_id > 0 {
                let prev_session_id = current_session_id - 1;
                match self.storage.get(prev_session_id).await {
                    Ok(Some(assignations)) => {
                        let membership = self
                            .assignations_to_membership(assignations, prev_session_id)
                            .await?;
                        self.previous_membership = Some(membership);
                    }
                    Ok(None) => {
                        self.previous_membership = Some(Membership::empty(prev_session_id));
                    }
                    Err(e) => return Err(OpinionError::Error(e)),
                }
            } else {
                // current_session_id is 0, so create empty previous
                self.previous_membership = Some(Membership::empty(0));
            }
        }

        // Generate opinions from current and previous memberships
        let result = self.generate_opinions();

        // Rotate memberships: previous <- current, current <- new
        self.previous_membership = self.current_membership.take();
        self.current_membership = Some(Membership::from(new_membership));

        // Reset opinion maps for next round
        self.reset_opinion_maps();

        result
    }

    fn generate_opinions(&self) -> Result<Opinions, OpinionError> {
        // Both memberships are guaranteed to be Some at this point
        let current = self.current_membership.as_ref().unwrap();
        let previous = self.previous_membership.as_ref().unwrap();

        // Generate new opinions from current membership
        let new_opinions = if current.is_empty() {
            // Non-operational session - return empty BitVec
            BitVec::new()
        } else {
            match self.peer_opinions_to_provider_bitvec(
                current.members().into_iter(),
                &self.positive_opinions,
                &self.negative_opinions,
                &current.provider_mappings,
                true, // Include self
            ) {
                Ok(bv) => bv,
                Err(e) => return Err(OpinionError::Error(e)),
            }
        };

        // Generate old opinions from previous membership
        let old_opinions = if previous.is_empty() {
            // Previous session was non-operational or genesis
            BitVec::new()
        } else {
            let include_self_in_old = previous.members().contains(&self.local_peer_id);

            match self.peer_opinions_to_provider_bitvec(
                previous.members().into_iter(),
                &self.prev_session_positive_opinions,
                &self.prev_session_negative_opinions,
                &previous.provider_mappings,
                include_self_in_old,
            ) {
                Ok(bv) => bv,
                Err(e) => return Err(OpinionError::Error(e)),
            }
        };

        Ok(Opinions {
            session_id: current.session_id,
            new_opinions,
            old_opinions,
        })
    }

    async fn assignations_to_membership(
        &self,
        assignations: Assignations<PeerId, SubnetworkId>,
        session_id: SessionNumber,
    ) -> Result<Membership, DynError> {
        let mut provider_mappings = HashMap::new();

        // Collect all unique peer IDs from all subnetworks
        let all_peer_ids: HashSet<PeerId> = assignations
            .values()
            .flat_map(|peers_set| peers_set.iter().copied())
            .collect();

        // Get provider mappings for each peer
        for peer_id in &all_peer_ids {
            if let Some(provider_id) = self.storage.get_provider_id(*peer_id).await? {
                provider_mappings.insert(*peer_id, provider_id);
            }
        }

        Ok(Membership {
            session_id,
            peers: all_peer_ids,
            provider_mappings,
        })
    }

    // Helper method to reset opinion maps (if you don't have it already)
    fn reset_opinion_maps(&mut self) {
        self.positive_opinions.clear();
        self.negative_opinions.clear();
        self.blacklist.clear();
        self.prev_session_positive_opinions.clear();
        self.prev_session_negative_opinions.clear();
        self.old_blacklist.clear();

        if let Some(ref membership) = self.current_membership {
            for peer_id in membership.members() {
                self.positive_opinions.insert(peer_id, 0);
                self.negative_opinions.insert(peer_id, 0);
            }
        }

        if let Some(ref membership) = self.previous_membership {
            for peer_id in membership.members() {
                self.prev_session_positive_opinions.insert(peer_id, 0);
                self.prev_session_negative_opinions.insert(peer_id, 0);
            }
        }
    }

    fn peer_opinions_to_provider_bitvec(
        &self,
        peers: impl Iterator<Item = PeerId>,
        positive: &HashMap<PeerId, u32>,
        negative: &HashMap<PeerId, u32>,
        provider_mappings: &HashMap<PeerId, ProviderId>,
        include_self: bool,
    ) -> Result<BitVec, DynError> {
        // BTreeMap so it is sorted
        let mut provider_opinions: BTreeMap<ProviderId, bool> = BTreeMap::new();

        for peer_id in peers {
            let provider_id = if peer_id == self.local_peer_id {
                self.local_provider_id
            } else {
                *provider_mappings.get(&peer_id).ok_or_else(|| {
                    DynError::from(format!("ProviderId not found for PeerId {peer_id}"))
                })?
            };

            let opinion = if include_self && peer_id == self.local_peer_id {
                true
            } else {
                let pos = positive.get(&peer_id).copied().unwrap_or(0);
                let neg = negative.get(&peer_id).copied().unwrap_or(0);
                pos > neg * OPINION_THRESHOLD
            };

            provider_opinions.insert(provider_id, opinion);
        }

        let bitvec: BitVec = provider_opinions.values().copied().collect();
        Ok(bitvec)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ed25519_dalek::SigningKey;
    use rand::{RngCore, SeedableRng as _, rngs::SmallRng};
    use subnetworks_assignations::{
        MembershipCreator as _, MembershipHandler as _,
        versions::history_aware_refill::HistoryAware,
    };

    use super::*;
    use crate::storage::adapters::mock::MockStorage;

    const REPLICATION_FACTOR: usize = 3;
    const SUBNETWORK_COUNT: usize = 16;

    fn create_test_peers(count: usize, rng: &mut impl RngCore) -> Vec<(PeerId, ProviderId)> {
        std::iter::repeat_with(|| {
            let mut seed = [0u8; 32];
            rng.fill_bytes(&mut seed);
            let signing_key = SigningKey::from_bytes(&seed);
            let verifying_key = signing_key.verifying_key();
            let provider_id = ProviderId(verifying_key);

            // Create libp2p keypair from the same seed bytes
            let secret_key = libp2p::identity::ed25519::SecretKey::try_from_bytes(seed)
                .expect("Valid ed25519 secret key");
            let keypair = libp2p::identity::ed25519::Keypair::from(secret_key);
            let peer_id = PeerId::from(libp2p::identity::PublicKey::from(keypair.public()));

            (peer_id, provider_id)
        })
        .take(count)
        .collect()
    }

    #[tokio::test]
    async fn test_session_0_genesis() {
        let mut rng = SmallRng::seed_from_u64(42);
        let storage = Arc::new(MockStorage::default());

        let peers = create_test_peers(50, &mut rng);
        let (local_peer_id, local_provider_id) = peers[0];

        let mut aggregator =
            OpinionAggregator::new(Arc::clone(&storage), local_peer_id, local_provider_id);

        // Create SubnetworkPeers for session 0
        let subnet_peers = SubnetworkPeers {
            session_id: 0,
            peers: peers
                .iter()
                .map(|(p, _)| (*p, libp2p::Multiaddr::empty()))
                .collect(),
            provider_mappings: peers.iter().map(|(p, prov)| (*p, *prov)).collect(),
        };

        // Session 0 - should return InsufficientData
        let result = aggregator.handle_session_change(&subnet_peers).await;
        assert!(matches!(result, Err(OpinionError::InsufficientData)));
    }

    #[tokio::test]
    async fn test_regular_flow_after_session_0() {
        let mut rng = SmallRng::seed_from_u64(42);
        let storage = Arc::new(MockStorage::default());

        let peers = create_test_peers(50, &mut rng);
        let (local_peer_id, local_provider_id) = peers[0];

        let mut aggregator =
            OpinionAggregator::new(Arc::clone(&storage), local_peer_id, local_provider_id);

        let mappings: HashMap<PeerId, ProviderId> = peers
            .iter()
            .map(|(peer, provider)| (*peer, *provider))
            .collect();

        let peer_ids: HashSet<PeerId> = peers.iter().map(|(p, _)| *p).collect();

        // Session 0 - store it
        let base_membership = HistoryAware::new(0, SUBNETWORK_COUNT, REPLICATION_FACTOR);
        let membership0 = base_membership.update(0, peer_ids.clone(), &mut rng);
        storage
            .store(0, membership0.subnetworks(), mappings.clone())
            .await
            .unwrap();

        // Create SubnetworkPeers for session 0
        let subnet_peers_0 = SubnetworkPeers {
            session_id: 0,
            peers: peers
                .iter()
                .map(|(p, _)| (*p, libp2p::Multiaddr::empty()))
                .collect(),
            provider_mappings: mappings.clone(),
        };

        // First call with session 0 - should return InsufficientData
        let result = aggregator.handle_session_change(&subnet_peers_0).await;
        assert!(matches!(result, Err(OpinionError::InsufficientData)));

        // Session 1 - store it
        let membership1 = membership0.update(1, peer_ids.clone(), &mut rng);
        storage
            .store(1, membership1.subnetworks(), mappings.clone())
            .await
            .unwrap();

        // Create SubnetworkPeers for session 1
        let subnet_peers_1 = SubnetworkPeers {
            session_id: 1,
            peers: peers
                .iter()
                .map(|(p, _)| (*p, libp2p::Multiaddr::empty()))
                .collect(),
            provider_mappings: mappings.clone(),
        };

        // Second call with session 1 - should generate opinions for session 0
        let result = aggregator.handle_session_change(&subnet_peers_1).await;
        match result {
            Ok(opinions) => {
                assert_eq!(opinions.session_id, 0);
                assert_eq!(opinions.new_opinions.len(), peers.len());
                assert_eq!(opinions.old_opinions.len(), 0); // No previous for session 0
            }
            Err(e) => panic!("Unexpected error: {e}"),
        }

        // Session 2
        let membership2 = membership1.update(2, peer_ids, &mut rng);
        storage
            .store(2, membership2.subnetworks(), mappings.clone())
            .await
            .unwrap();

        let subnet_peers_2 = SubnetworkPeers {
            session_id: 2,
            peers: peers
                .iter()
                .map(|(p, _)| (*p, libp2p::Multiaddr::empty()))
                .collect(),
            provider_mappings: mappings,
        };

        // Third call - should generate opinions for session 1
        let result = aggregator.handle_session_change(&subnet_peers_2).await;
        match result {
            Ok(opinions) => {
                assert_eq!(opinions.session_id, 1);
                assert_eq!(opinions.new_opinions.len(), peers.len());
                assert_eq!(opinions.old_opinions.len(), peers.len()); // Now has previous
            }
            Err(e) => panic!("Unexpected error: {e}"),
        }
    }

    #[tokio::test]
    async fn test_restart_at_session_greater_than_0() {
        let mut rng = SmallRng::seed_from_u64(42);
        let storage = Arc::new(MockStorage::default());

        let peers = create_test_peers(50, &mut rng);
        let (local_peer_id, local_provider_id) = peers[0];

        let mappings: HashMap<PeerId, ProviderId> = peers
            .iter()
            .map(|(peer, provider)| (*peer, *provider))
            .collect();

        let peer_ids: HashSet<PeerId> = peers.iter().map(|(p, _)| *p).collect();
        let base_membership = HistoryAware::new(1, SUBNETWORK_COUNT, REPLICATION_FACTOR);

        // Store sessions 3 and 4 in storage (simulating they were stored before
        // restart)
        let membership3 = base_membership.update(3, peer_ids.clone(), &mut rng);
        let membership4 = membership3.update(4, peer_ids.clone(), &mut rng);

        storage
            .store(3, membership3.subnetworks(), mappings.clone())
            .await
            .unwrap();
        storage
            .store(4, membership4.subnetworks(), mappings.clone())
            .await
            .unwrap();

        // Create new aggregator (simulating restart)
        let mut aggregator =
            OpinionAggregator::new(Arc::clone(&storage), local_peer_id, local_provider_id);

        // Create SubnetworkPeers for session 5 (new session after restart)
        let subnet_peers_5 = SubnetworkPeers {
            session_id: 5,
            peers: peers
                .iter()
                .map(|(p, _)| (*p, libp2p::Multiaddr::empty()))
                .collect(),
            provider_mappings: mappings,
        };

        // Handle session change with session 5 (restart scenario)
        // This should restore session 4 as current and session 3 as previous,
        // then generate opinions for session 4
        let result = aggregator.handle_session_change(&subnet_peers_5).await;

        match result {
            Ok(opinions) => {
                assert_eq!(opinions.session_id, 4); // Should generate opinions for session 4
                assert_eq!(opinions.new_opinions.len(), peers.len());
                assert_eq!(opinions.old_opinions.len(), peers.len());
            }
            Err(e) => panic!("Unexpected error: {e}"),
        }
    }

    #[tokio::test]
    async fn test_non_operational_session() {
        let mut rng = SmallRng::seed_from_u64(42);
        let storage = Arc::new(MockStorage::default());

        let peers = create_test_peers(50, &mut rng);
        let (local_peer_id, local_provider_id) = peers[0];

        let mut aggregator =
            OpinionAggregator::new(Arc::clone(&storage), local_peer_id, local_provider_id);

        let mappings: HashMap<PeerId, ProviderId> = peers
            .iter()
            .map(|(peer, provider)| (*peer, *provider))
            .collect();

        let peer_ids: HashSet<PeerId> = peers.iter().map(|(p, _)| *p).collect();
        let base_membership = HistoryAware::new(1, SUBNETWORK_COUNT, REPLICATION_FACTOR);

        // Session 1 - operational
        let membership1 = base_membership.update(1, peer_ids.clone(), &mut rng);
        storage
            .store(1, membership1.subnetworks(), mappings.clone())
            .await
            .unwrap();

        let subnet_peers_1 = SubnetworkPeers {
            session_id: 1,
            peers: peers
                .iter()
                .map(|(p, _)| (*p, libp2p::Multiaddr::empty()))
                .collect(),
            provider_mappings: mappings.clone(),
        };

        // Store session 0 as well for the test
        let membership0 = base_membership.update(0, peer_ids.clone(), &mut rng);
        storage
            .store(0, membership0.subnetworks(), mappings.clone())
            .await
            .unwrap();

        aggregator
            .handle_session_change(&subnet_peers_1)
            .await
            .unwrap();

        // Session 2 - non-operational (empty)
        // Store empty assignations for session 2
        storage
            .store(2, HashMap::new(), HashMap::new())
            .await
            .unwrap();

        let subnet_peers_2 = SubnetworkPeers {
            session_id: 2,
            peers: HashMap::new(), // Empty - non-operational
            provider_mappings: HashMap::new(),
        };

        let result = aggregator.handle_session_change(&subnet_peers_2).await;
        match result {
            Ok(opinions) => {
                assert_eq!(opinions.session_id, 1);
                assert_eq!(opinions.new_opinions.len(), peers.len()); // Session 1 was operational
                assert_eq!(opinions.old_opinions.len(), peers.len()); // Session 0 was operational
            }
            Err(e) => panic!("Unexpected error: {e}"),
        }

        // Session 3 - back to operational
        let membership3 = base_membership.update(3, peer_ids, &mut rng);
        storage
            .store(3, membership3.subnetworks(), mappings.clone())
            .await
            .unwrap();

        let subnet_peers_3 = SubnetworkPeers {
            session_id: 3,
            peers: peers
                .iter()
                .map(|(p, _)| (*p, libp2p::Multiaddr::empty()))
                .collect(),
            provider_mappings: mappings,
        };

        let result = aggregator.handle_session_change(&subnet_peers_3).await;
        match result {
            Ok(opinions) => {
                assert_eq!(opinions.session_id, 2);
                assert_eq!(opinions.new_opinions.len(), 0); // Session 2 was non-operational
                assert_eq!(opinions.old_opinions.len(), peers.len()); // Session 1 was operational
            }
            Err(e) => panic!("Unexpected error: {e}"),
        }
    }
}
