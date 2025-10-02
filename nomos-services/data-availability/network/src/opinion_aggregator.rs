use std::collections::{BTreeMap, HashMap, HashSet};

use bitvec::prelude::*;
use libp2p::PeerId;
use nomos_core::{block::SessionNumber, sdp::ProviderId};
use nomos_da_network_core::{
    SubnetworkId,
    protocols::sampling::opinions::{Opinion, OpinionEvent},
};
use overwatch::DynError;
use subnetworks_assignations::MembershipHandler;

use crate::storage::MembershipStorageAdapter;

const OPINION_THRESHOLD: u32 = 10;

pub struct OpinionAggregator<Membership, Storage> {
    storage: Storage,

    local_peer_id: PeerId,
    local_provider_id: ProviderId,

    positive_opinions: HashMap<PeerId, u32>,
    negative_opinions: HashMap<PeerId, u32>,
    blacklist: HashSet<PeerId>,

    old_positive_opinions: HashMap<PeerId, u32>,
    old_negative_opinions: HashMap<PeerId, u32>,
    old_blacklist: HashSet<PeerId>,

    current_membership: Option<Membership>,
    previous_membership: Option<Membership>,
}

#[derive(Debug)]
pub struct Opinions {
    pub session_id: SessionNumber,
    pub new_opinions: BitVec,
    pub old_opinions: BitVec,
}

impl<Membership, Storage> OpinionAggregator<Membership, Storage>
where
    Membership: MembershipHandler<Id = PeerId> + Send + Sync,
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
            old_positive_opinions: HashMap::new(),
            old_negative_opinions: HashMap::new(),
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
            && current.session_id() == session_id
        {
            if let Some(count) = self.positive_opinions.get_mut(&peer_id) {
                *count += 1;
            }
            return;
        }
        if let Some(ref previous) = self.previous_membership
            && previous.session_id() == session_id
            && let Some(count) = self.old_positive_opinions.get_mut(&peer_id)
        {
            *count += 1;
        }
    }

    fn handle_negative(&mut self, peer_id: PeerId, session_id: SessionNumber) {
        if let Some(ref current) = self.current_membership
            && current.session_id() == session_id
        {
            if let Some(count) = self.negative_opinions.get_mut(&peer_id) {
                *count += 1;
            }
            return;
        }
        if let Some(ref previous) = self.previous_membership
            && previous.session_id() == session_id
            && let Some(count) = self.old_negative_opinions.get_mut(&peer_id)
        {
            *count += 1;
        }
    }

    fn handle_blacklist(&mut self, peer_id: PeerId, session_id: SessionNumber) {
        if let Some(ref current) = self.current_membership
            && current.session_id() == session_id
        {
            if let Some(count) = self.positive_opinions.get_mut(&peer_id) {
                *count = 0;
            }
            self.blacklist.insert(peer_id);
            return;
        }
        if let Some(ref previous) = self.previous_membership
            && previous.session_id() == session_id
        {
            if let Some(count) = self.old_positive_opinions.get_mut(&peer_id) {
                *count = 0;
            }
            self.old_blacklist.insert(peer_id);
        }
    }

    pub async fn handle_session_change(
        &mut self,
        new_membership: Membership,
    ) -> Result<Option<Opinions>, DynError> {
        self.previous_membership = self.current_membership.take();
        self.current_membership = Some(new_membership);

        let opinions = match (
            self.current_membership.as_ref(),
            self.previous_membership.as_ref(),
        ) {
            (Some(_), Some(_)) => Some(self.generate_opinions().await?),
            _ => None,
        };

        self.positive_opinions.clear();
        self.negative_opinions.clear();
        self.blacklist.clear();
        self.old_positive_opinions.clear();
        self.old_negative_opinions.clear();
        self.old_blacklist.clear();

        // Pre-populate opinion maps with zeros for the current membership
        if let Some(ref membership) = self.current_membership {
            for peer_id in membership.members() {
                self.positive_opinions.insert(peer_id, 0);
                self.negative_opinions.insert(peer_id, 0);
            }
        }

        // Pre-populate old opinion maps for the previous membership
        if let Some(ref membership) = self.previous_membership {
            for peer_id in membership.members() {
                self.old_positive_opinions.insert(peer_id, 0);
                self.old_negative_opinions.insert(peer_id, 0);
            }
        }

        Ok(opinions)
    }

    async fn generate_opinions(&self) -> Result<Opinions, DynError> {
        let current = self
            .current_membership
            .as_ref()
            .ok_or_else(|| DynError::from("No current membership"))?;
        let previous = self
            .previous_membership
            .as_ref()
            .ok_or_else(|| DynError::from("No previous membership"))?;

        let include_self_in_old = previous
            .members()
            .into_iter()
            .any(|id| id == self.local_peer_id);

        let new_opinions = self
            .peer_opinions_to_provider_bitvec(
                current.members().into_iter(),
                &self.positive_opinions,
                &self.negative_opinions,
                true, // Always include self in new_opinions
            )
            .await?;

        let old_opinions = self
            .peer_opinions_to_provider_bitvec(
                previous.members().into_iter(),
                &self.old_positive_opinions,
                &self.old_negative_opinions,
                include_self_in_old,
            )
            .await?;

        Ok(Opinions {
            session_id: current.session_id(),
            new_opinions,
            old_opinions,
        })
    }

    async fn peer_opinions_to_provider_bitvec(
        &self,
        peers: impl Iterator<Item = PeerId>,
        positive: &HashMap<PeerId, u32>,
        negative: &HashMap<PeerId, u32>,
        include_self: bool,
    ) -> Result<BitVec, DynError> {
        // BtreeMap so it is sorted
        let mut provider_opinions: BTreeMap<ProviderId, bool> = BTreeMap::new();

        for peer_id in peers {
            // Get ProviderId from storage
            let provider_id = if peer_id == self.local_peer_id {
                self.local_provider_id
            } else if let Some(pid) = self.storage.get_provider_id(peer_id).await? {
                pid
            } else {
                tracing::warn!("No ProviderId found for PeerId {}, skipping", peer_id);
                continue;
            };

            // Calculate opinion
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
        MembershipCreator as _, versions::history_aware_refill::HistoryAware,
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
    async fn test_opinions_with_realistic_membership() {
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
        let membership1 = base_membership.update(1, peer_ids.clone(), &mut rng);

        storage
            .store(1, membership1.subnetworks(), mappings.clone())
            .await
            .unwrap();

        // First session
        let result = aggregator.handle_session_change(membership1.clone()).await; // ← Also add .clone() here
        assert!(result.unwrap().is_none());

        // Second session - FIX: update FROM membership1, not base_membership
        let membership2 = membership1.update(2, peer_ids, &mut rng);
        storage
            .store(2, membership2.subnetworks(), mappings)
            .await
            .unwrap();

        let result = aggregator.handle_session_change(membership2).await;
        assert!(result.is_ok());

        let opinions = result.unwrap();
        assert!(opinions.is_some());

        let opinions = opinions.unwrap();
        assert_eq!(opinions.session_id, 2);
        assert_eq!(opinions.new_opinions.len(), peers.len());
        assert_eq!(opinions.old_opinions.len(), peers.len());
    }
}
