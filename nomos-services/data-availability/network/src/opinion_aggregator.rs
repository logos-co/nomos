use std::collections::{HashMap, HashSet};

use libp2p::PeerId;
use nomos_core::block::SessionNumber;
use nomos_da_network_core::protocols::sampling::opinions::{Opinion, OpinionEvent};
use subnetworks_assignations::MembershipHandler;

const OPINION_THRESHOLD: f32 = 0.9;

pub struct OpinionAggregator<Membership>
where
    Membership: MembershipHandler<Id = PeerId>,
{
    // Opinion counters for current session (new blocks)
    positive_opinions: HashMap<PeerId, u32>,
    negative_opinions: HashMap<PeerId, u32>,
    blacklist: HashSet<PeerId>,

    // Opinion counters for previous session (old blocks)
    old_positive_opinions: HashMap<PeerId, u32>,
    old_negative_opinions: HashMap<PeerId, u32>,
    old_blacklist: HashSet<PeerId>,

    // Memberships contain both the peer lists and session IDs
    current_membership: Option<Membership>,
    previous_membership: Option<Membership>,
}

impl<Membership> OpinionAggregator<Membership>
where
    Membership: MembershipHandler<Id = PeerId>,
{
    pub fn new() -> Self {
        Self {
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

    pub fn on_session_change(&mut self, new_membership: Membership) {
        // Generate activity proof here if we have both memberships (before clearing)
        if self.current_membership.is_some() && self.previous_membership.is_some() {
            // This would be session s+1, time to generate proof for session s
            // TODO: generate_activity_proof() and send it
        }

        self.previous_membership = self.current_membership.take();

        self.positive_opinions.clear();
        self.negative_opinions.clear();
        self.blacklist.clear();
        self.old_positive_opinions.clear();
        self.old_negative_opinions.clear();
        self.old_blacklist.clear();

        // Pre-populate opinion maps with zeros for new membership so the vectors fit
        for peer_id in new_membership.members() {
            self.positive_opinions.insert(peer_id, 0);
            self.negative_opinions.insert(peer_id, 0);
        }

        // Pre-populate for previous membership (if it exists) so the vectors fit
        if let Some(ref membership) = self.previous_membership {
            for peer_id in membership.members() {
                self.old_positive_opinions.insert(peer_id, 0);
                self.old_negative_opinions.insert(peer_id, 0);
            }
        }

        self.current_membership = Some(new_membership);
    }
}
