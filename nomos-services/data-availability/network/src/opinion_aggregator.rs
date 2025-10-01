use std::collections::{BTreeMap, HashMap, HashSet};

use bitvec::prelude::*;
use libp2p::PeerId;
use nomos_core::{block::SessionNumber, sdp::ProviderId};
use nomos_da_network_core::protocols::sampling::opinions::{Opinion, OpinionEvent};
use subnetworks_assignations::MembershipHandler;

const OPINION_THRESHOLD: u32 = 10;

pub struct OpinionAggregator<Membership>
where
    Membership: MembershipHandler<Id = PeerId>,
{
    local_peer_id: PeerId,
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
#[expect(
    dead_code,
    reason = "Will be used when SDP adapter integration is complete"
)]
pub struct Opinions {
    pub session_id: SessionNumber,
    pub new_opinions: HashMap<PeerId, bool>,
    pub old_opinions: HashMap<PeerId, bool>,
}

impl<Membership> OpinionAggregator<Membership>
where
    Membership: MembershipHandler<Id = PeerId>,
{
    pub fn new(local_peer_id: PeerId) -> Self {
        Self {
            local_peer_id,
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

    pub fn handle_session_change(&mut self, new_membership: Membership) -> Option<Opinions> {
        // Generate opinions before clearing if we have both memberships
        let opinions = if self.current_membership.is_some() && self.previous_membership.is_some() {
            self.generate_opinions()
        } else {
            None
        };

        self.previous_membership = self.current_membership.take();

        self.positive_opinions.clear();
        self.negative_opinions.clear();
        self.blacklist.clear();
        self.old_positive_opinions.clear();
        self.old_negative_opinions.clear();
        self.old_blacklist.clear();

        // Pre-populate opinion maps with zeros to the right vector size
        for peer_id in new_membership.members() {
            self.positive_opinions.insert(peer_id, 0);
            self.negative_opinions.insert(peer_id, 0);
        }

        if let Some(ref membership) = self.previous_membership {
            for peer_id in membership.members() {
                self.old_positive_opinions.insert(peer_id, 0);
                self.old_negative_opinions.insert(peer_id, 0);
            }
        }

        self.current_membership = Some(new_membership);

        // Return opinions to be sent to SDP
        // SDP will convert peerID -> ProviderID and sort in lexicographical order
        opinions
    }

    pub fn generate_opinions(&self) -> Option<Opinions> {
        let current = self.current_membership.as_ref()?;
        let previous = self.previous_membership.as_ref()?;

        let new_opinions = self.calculate_opinions_map(
            current.members().into_iter(),
            &self.positive_opinions,
            &self.negative_opinions,
            true, // Always include self peer id in new_opinions, per spec
        );

        let old_opinions = self.calculate_opinions_map(
            previous.members().into_iter(),
            &self.old_positive_opinions,
            &self.old_negative_opinions,
            previous
                .members()
                .into_iter()
                .any(|id| id == self.local_peer_id), /* only include local peer id in old
                                                      * opinions if it was participating in the
                                                      * previous session */
        );

        Some(Opinions {
            session_id: current.session_id(),
            new_opinions,
            old_opinions,
        })
    }

    fn calculate_opinions_map(
        &self,
        peers: impl Iterator<Item = PeerId>,
        positive: &HashMap<PeerId, u32>,
        negative: &HashMap<PeerId, u32>,
        include_self: bool,
    ) -> HashMap<PeerId, bool> {
        peers
            .map(|peer_id| {
                let opinion = if include_self && peer_id == self.local_peer_id {
                    true
                } else {
                    let pos = positive.get(&peer_id).copied().unwrap_or(0);
                    let neg = negative.get(&peer_id).copied().unwrap_or(0);
                    pos > neg * OPINION_THRESHOLD
                };
                (peer_id, opinion)
            })
            .collect()
    }
}

impl Opinions {
    #[expect(
        dead_code,
        reason = "Will be used by SDP adapter for generating activity proofs"
    )]
    pub fn to_bit_vectors(
        new_opinions: &BTreeMap<ProviderId, bool>,
        old_opinions: &BTreeMap<ProviderId, bool>,
    ) -> (BitVec, BitVec) {
        // Values are already in sorted order by ProviderId
        let new_bits: BitVec = new_opinions.values().copied().collect();
        let old_bits: BitVec = old_opinions.values().copied().collect();

        (new_bits, old_bits)
    }
}
