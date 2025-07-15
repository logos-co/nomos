use std::{
    cmp::Reverse,
    collections::{BTreeSet, BinaryHeap},
};

use nomos_sdp_core::DeclarationId;
use nomos_utils::fisheryates::FisherYates;
use participant::Participant;
use rand::RngCore;
use subnetwork::Subnetwork;

pub mod participant;
pub mod subnetwork;

type Assignations = Vec<BTreeSet<DeclarationId>>;

// Minimum binary heap as by default is ordered as a max heap
type Subnetworks = BinaryHeap<Reverse<Subnetwork>>;

// Minimum binary heap as by default is ordered as a max heap
type Participants = BinaryHeap<Reverse<Participant>>;

pub struct HistoryAwareRefill;

impl HistoryAwareRefill {
    fn subnetworks_filled_up_to_replication_factor(
        subnetworks: &Subnetworks,
        replication_factor: usize,
    ) -> bool {
        subnetworks
            .iter()
            .all(|Reverse(subnetwork)| subnetwork.len() >= replication_factor)
    }

    fn all_nodes_assigned(participants: &Participants, average_participation: usize) -> bool {
        participants
            .iter()
            .all(|participant| participant.0.participation >= average_participation)
    }

    fn heap_pop_next_for_subnetwork(
        subnetwork: &Subnetwork,
        participants: &mut Participants,
    ) -> Participant {
        let mut poped = BinaryHeap::new();
        while let Some(Reverse(participant)) = participants.pop() {
            if !subnetwork.participants.contains(&participant) {
                participants.append(&mut poped);
                return participant;
            }
            poped.push(Reverse(participant));
        }
        unreachable!("It should never reach this state unless not catching invariants before hand");
    }

    fn balance_subnetwork_shrink<Rng: RngCore>(
        subnetworks: impl IntoIterator<Item = Subnetwork>,
        rng: &mut Rng,
    ) -> impl IntoIterator<Item = Subnetwork> {
        let mut subnetworks: Vec<_> = subnetworks.into_iter().collect();
        let first = 0usize;
        let last = subnetworks.len() - 1;
        loop {
            subnetworks.sort();
            let diff_count = {
                let max = subnetworks.first().expect("non empty subnetworks set");
                let min = subnetworks.last().expect("non empty subnetworks set");
                let diff = max.len() - min.len();
                if diff <= 1 {
                    return subnetworks;
                }
                diff / 2
            };

            let [max, min] = subnetworks
                .get_disjoint_mut([first, last])
                .expect("subnetworks set is never less than 2");

            let diff: Vec<Participant> = FisherYates::sample(
                max.participants.difference(&min.participants),
                diff_count,
                rng,
            )
            .copied()
            .collect();
            for participant in diff {
                max.participants.remove(&participant);
                min.participants.insert(participant);
            }
        }
    }
}
