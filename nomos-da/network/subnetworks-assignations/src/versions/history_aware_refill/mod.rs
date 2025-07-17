use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeSet, BinaryHeap, HashSet},
};

use counter::Counter;
use nomos_sdp_core::DeclarationId;
use nomos_utils::fisheryates::FisherYates;
use participant::Participant;
use rand::RngCore;
use subnetwork::Subnetwork;

use crate::SubnetworkId;

mod participant;
mod subnetwork;

type Assignations = Vec<BTreeSet<DeclarationId>>;

// Minimum binary heap as by default is ordered as a max heap
type Subnetworks<'s> = BinaryHeap<Reverse<&'s mut Subnetwork>>;

// Minimum binary heap as by default is ordered as a max heap
type Participants<'p> = BinaryHeap<Reverse<&'p mut Participant>>;

pub struct HistoryAwareRefill;

impl HistoryAwareRefill {
    fn subnetworks_filled_up_to_replication_factor(
        subnetworks_lens: impl IntoIterator<Item = usize>,
        replication_factor: usize,
    ) -> bool {
        subnetworks_lens
            .into_iter()
            .all(|subnetwork_len| subnetwork_len >= replication_factor)
    }

    fn all_nodes_assigned(
        participation: impl IntoIterator<Item = usize>,
        average_participation: usize,
    ) -> bool {
        participation
            .into_iter()
            .all(|participation| participation >= average_participation)
    }

    fn heap_pop_next_for_subnetwork<'o>(
        subnetwork: &Subnetwork,
        participants: &mut Participants<'o>,
    ) -> &'o mut Participant {
        let mut poped = BinaryHeap::new();
        while let Some(Reverse(participant)) = participants.pop() {
            if !subnetwork
                .participants
                .contains(&participant.declaration_id)
            {
                participants.append(&mut poped);
                return participant;
            }
            poped.push(Reverse(participant));
        }
        unreachable!("It should never reach this state unless not catching invariants before hand");
    }

    fn balance_subnetwork_shrink<'s, Rng: RngCore>(
        subnetworks: impl IntoIterator<Item = &'s mut Subnetwork>,
        rng: &mut Rng,
    ) {
        let mut subnetworks: Vec<_> = subnetworks.into_iter().collect();
        let first = 0usize;
        let last = subnetworks.len() - 1;
        loop {
            // not the most efficient, but it's a constant cost because subnetwork count
            // does not change
            subnetworks.sort();

            let diff_count = {
                let max = subnetworks[last].len();
                let min = subnetworks[first].len();
                let diff = max - min;
                if diff <= 1 {
                    break;
                }
                diff / 2
            };

            let [max, min] = subnetworks
                .get_disjoint_mut([last, first])
                .expect("subnetworks set is never less than 2");

            let diff: Vec<DeclarationId> = FisherYates::sample(
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

    fn balance_subnetwork_grow<'i, Rng: RngCore>(
        subnetworks: impl IntoIterator<Item = &'i mut Subnetwork>,
        participants: impl IntoIterator<Item = &'i mut Participant>,
        average_participation: usize,
        rng: &mut Rng,
    ) {
        let mut subnetworks: Vec<_> = subnetworks.into_iter().collect();
        subnetworks.sort();
        let mut participants: Vec<_> = participants.into_iter().collect();
        participants.sort();

        let participants_to_balance: Vec<usize> = participants
            .iter()
            .enumerate()
            .filter_map(|(i, participant)| {
                (participant.participation > average_participation).then_some(i)
            })
            .collect(); // have to collect to avoid borrowing as later is needed to borrow as mut
        for participant in participants_to_balance {
            let participant = participants
                .get_mut(participant)
                .expect("Participant was present when filtering above");

            let member_subnetworks: Vec<usize> = subnetworks
                .iter()
                .enumerate()
                .filter_map(|(i, subnetwork)| {
                    subnetwork
                        .participants
                        .contains(&participant.declaration_id)
                        .then_some(i)
                })
                .collect(); // have to collect to avoid borrowing as later is needed to borrow as mut

            let destinations = FisherYates::sample(
                member_subnetworks,
                participant.participation - average_participation,
                rng,
            );
            for subnetwork in destinations {
                subnetworks[subnetwork]
                    .participants
                    .remove(&participant.declaration_id);
                participant.participation -= 1;
            }
        }
    }

    fn fill_subnetworks<'i>(
        participants: impl IntoIterator<Item = &'i mut Participant>,
        subnetworks: impl IntoIterator<Item = &'i mut Subnetwork>,
        average_participation: usize,
        replication_factor: usize,
    ) {
        let mut participants: Participants = participants.into_iter().map(Reverse).collect();
        let mut subnetworks: Subnetworks = subnetworks.into_iter().map(Reverse).collect();
        loop {
            let subnetworks_filled_up_to_replication_factor =
                Self::subnetworks_filled_up_to_replication_factor(
                    subnetworks.iter().map(|Reverse(s)| s.len()),
                    replication_factor,
                );

            let all_nodes_assigned = Self::all_nodes_assigned(
                participants.iter().map(|Reverse(p)| p.participation),
                average_participation,
            );

            if subnetworks_filled_up_to_replication_factor && all_nodes_assigned {
                break;
            }

            let subnetwork = subnetworks.pop().expect("Subnetworks are never empty").0;
            let participant = Self::heap_pop_next_for_subnetwork(subnetwork, &mut participants);

            subnetwork.participants.insert(participant.declaration_id);
            participant.participation += 1;
            subnetworks.push(Reverse(subnetwork));
            participants.push(Reverse(participant));
        }
    }

    pub fn calculate_subnetwork_assignations<Rng: RngCore>(
        new_nodes_list: &[DeclarationId],
        previous_subnets: Assignations,
        replication_factor: usize,
        rng: &mut Rng,
    ) -> Assignations {
        assert!(
            new_nodes_list.len() >= replication_factor,
            "The network size is smaller than the replication factor"
        );
        // The algorithm works as follows:
        // 1. Remove nodes that are not active from the previous subnetworks
        //    assignations
        // 2. If the network is decreasing (less available nodes than previous nodes),
        //    balance subnetworks:
        //    1) Until the biggest subnetwork and the smallest subnetwork size
        //       difference is <= 1
        //    2) Pick the biggest subnetwork and migrate a random half of the node
        //       difference to the smallest subnetwork, randomly choosing them.
        // 3. If the network is increasing (more available nodes than previous nodes),
        //    balance subnetworks:
        // 1) For each (sorted) participant, remove the participant from random
        //    subnetworks (coming from sorted list) until the participation of is equal
        //    to the average participation.
        // 4. Create a heap with the set of active nodes ordered by, primary the number
        //    of subnetworks each participant is at and secondary by the DeclarationId
        //    of the participant (ascending order).
        // 5. Create a heap with the subnetworks ordered by the number of participants
        //    in each subnetwork
        // 6. Until all subnetworks are filled up to a replication factor and all nodes
        //    are assigned:
        //      1) pop the subnetwork with the fewest participants
        //      2) pop the participant with less participation
        //      3) push the participant into the subnetwork and increment its
        //      participation count
        //      4) push the participant and the subnetwork into the respective heaps
        // 7. Return the subnetworks ordered by its subnetwork id

        let average_participation =
            (previous_subnets.len() * replication_factor / new_nodes_list.len()).max(1);

        let previous_nodes: BTreeSet<_> = previous_subnets.iter().flatten().copied().collect();
        let new_nodes: BTreeSet<_> = new_nodes_list.iter().copied().collect();
        let unavailable_nodes: BTreeSet<_> =
            previous_nodes.difference(&new_nodes).copied().collect();

        let active_assignations: Assignations = previous_subnets
            .into_iter()
            .map(|subnet| subnet.difference(&unavailable_nodes).copied().collect())
            .collect();

        let assigned_count: Counter<_> = active_assignations.iter().flatten().collect();
        let mut available_nodes: Vec<Participant> = new_nodes
            .iter()
            .map(|id| Participant {
                participation: assigned_count.get(id).copied().unwrap_or_default(),
                declaration_id: *id,
            })
            .collect();

        let mut subnetworks: Vec<Subnetwork> = active_assignations
            .into_iter()
            .enumerate()
            .map(|(subnetwork_id, participants)| Subnetwork {
                participants,
                subnetwork_id: subnetwork_id as SubnetworkId,
            })
            .collect();

        match new_nodes.len().cmp(&previous_nodes.len()) {
            Ordering::Less => {
                Self::balance_subnetwork_shrink(subnetworks.iter_mut(), rng);
            }
            Ordering::Greater => {
                Self::balance_subnetwork_grow(
                    subnetworks.iter_mut(),
                    available_nodes.iter_mut(),
                    average_participation,
                    rng,
                );
            }
            Ordering::Equal => {}
        }

        Self::fill_subnetworks(
            available_nodes.iter_mut(),
            subnetworks.iter_mut(),
            average_participation,
            replication_factor,
        );

        subnetworks.sort_by_key(|subnetwork| subnetwork.subnetwork_id);
        subnetworks
            .into_iter()
            .map(|subnetwork| subnetwork.participants)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use rand::{rng, rngs::SmallRng, seq::IteratorRandom as _, Rng, SeedableRng};

    use super::*;

    const SUBNETWORK_SIZE: usize = 2048;
    const REPLICATION_FACTOR: usize = 5;
    const MIN_NETWORK_SIZE: usize = 40;

    fn assert_assignations(
        assignations: &Assignations,
        nodes: &[DeclarationId],
        replication_factor: usize,
    ) {
        assert_eq!(
            assignations.iter().flatten().collect::<HashSet<_>>().len(),
            nodes.len(),
            "Only active nodes should be assigned"
        );
        assert!(
            assignations
                .iter()
                .map(BTreeSet::len)
                .all(|len| len >= replication_factor),
            "Subnetworks should be filled up to the replication factor"
        );
        assert!(
            assignations.iter().map(BTreeSet::len).max().unwrap()
                - assignations.iter().map(BTreeSet::len).min().unwrap()
                <= 1,
            "Subnetwork size variant should not be bigger than 1",
        );
        let counter = assignations.iter().flatten().collect::<Counter<_>>();
        let sizes = counter.values().copied().collect::<HashSet<usize>>();
        assert!(
            sizes.len() <= 2,
            "Nodes should be assigned uniformly to subnetworks: \n{sizes:?} \n{counter:?}"
        );
    }

    fn mutate_nodes(nodes: &mut [DeclarationId], count: usize) {
        assert!(count <= nodes.len());
        let mut rng = rng();
        for i in (0..nodes.len()).choose_multiple(&mut rng, count) {
            let mut buff = [0u8; 32];
            rng.fill_bytes(&mut buff);
            nodes[i] = DeclarationId(buff);
        }
    }

    fn expand_nodes(
        nodes: &[DeclarationId],
        count: usize,
    ) -> impl Iterator<Item = DeclarationId> + '_ {
        nodes.iter().copied().chain(
            std::iter::repeat_with(|| {
                let mut rng = rng();
                let mut buff = [0u8; 32];
                rng.fill_bytes(&mut buff);
                DeclarationId(buff)
            })
            .take(count),
        )
    }

    fn shrink_nodes(
        nodes: &[DeclarationId],
        count: usize,
    ) -> impl Iterator<Item = DeclarationId> + '_ {
        let mut rng = rng();
        nodes
            .iter()
            .copied()
            .choose_multiple(&mut rng, count)
            .into_iter()
    }

    fn test_single_with<Rng: RngCore>(
        subnetwork_size: usize,
        replication_factor: usize,
        network_size: usize,
        rng: &mut Rng,
    ) -> Assignations {
        let nodes: Vec<DeclarationId> = std::iter::repeat_with(|| {
            let mut buff = [0u8; 32];
            rng.fill_bytes(&mut buff);
            DeclarationId(buff)
        })
        .take(network_size)
        .collect();

        let previous_nodes: Vec<BTreeSet<DeclarationId>> =
            std::iter::repeat_with(BTreeSet::<DeclarationId>::new)
                .take(subnetwork_size)
                .collect();

        let assignations = HistoryAwareRefill::calculate_subnetwork_assignations(
            &nodes,
            previous_nodes,
            replication_factor,
            rng,
        );
        assert_assignations(&assignations, &nodes, replication_factor);
        assignations
    }

    #[test]
    fn test_single_network_sizes() {
        for &size in &[100, 500, 1000, 10000, 100_000] {
            let mut rng = rng();
            test_single_with(SUBNETWORK_SIZE, REPLICATION_FACTOR, size, &mut rng);
        }
    }

    #[test]
    fn test_evolving_increasing_network() {
        let mut rng = rng();

        let nodes: Vec<DeclarationId> = std::iter::repeat_with(|| {
            let mut buff = [0u8; 32];
            rng.fill_bytes(&mut buff);
            DeclarationId(buff)
        })
        .take(100)
        .collect();

        let mut assignations: Vec<BTreeSet<DeclarationId>> = std::iter::repeat_with(BTreeSet::new)
            .take(SUBNETWORK_SIZE)
            .collect();

        assignations = HistoryAwareRefill::calculate_subnetwork_assignations(
            &nodes,
            assignations,
            REPLICATION_FACTOR,
            &mut rng,
        );
        assert_assignations(&assignations, &nodes, REPLICATION_FACTOR);

        let mut new_nodes = nodes.clone();

        for network_size in [300, 500, 1000, 10000, 100_000] {
            new_nodes = expand_nodes(&new_nodes, network_size - nodes.len()).collect();
            let third_networks_size = new_nodes.len() / 3;
            mutate_nodes(&mut new_nodes, third_networks_size);
            assignations = HistoryAwareRefill::calculate_subnetwork_assignations(
                &new_nodes,
                assignations,
                REPLICATION_FACTOR,
                &mut rng,
            );
            assert_assignations(&assignations, &new_nodes, REPLICATION_FACTOR);
        }
    }

    #[test]
    fn test_evolving_decreasing_network() {
        let mut rng = rng();

        let nodes: Vec<DeclarationId> = std::iter::repeat_with(|| {
            let mut buff = [0u8; 32];
            rng.fill_bytes(&mut buff);
            DeclarationId(buff)
        })
        .take(100_000)
        .collect();

        let mut assignations: Vec<BTreeSet<DeclarationId>> = std::iter::repeat_with(BTreeSet::new)
            .take(SUBNETWORK_SIZE)
            .collect();

        assignations = HistoryAwareRefill::calculate_subnetwork_assignations(
            &nodes,
            assignations,
            REPLICATION_FACTOR,
            &mut rng,
        );
        assert_assignations(&assignations, &nodes, REPLICATION_FACTOR);

        let mut new_nodes = nodes.clone();

        for network_size in [10000, 1000, 500, 300] {
            new_nodes = shrink_nodes(&new_nodes, network_size).collect();
            let third_networks_size = new_nodes.len() / 3;
            mutate_nodes(&mut new_nodes, third_networks_size);
            assignations = HistoryAwareRefill::calculate_subnetwork_assignations(
                &new_nodes,
                assignations,
                REPLICATION_FACTOR,
                &mut rng,
            );
            assert_assignations(&assignations, &new_nodes, REPLICATION_FACTOR);
        }
    }

    #[test]
    fn test_random_increasing_or_decreasing_network() {
        let mut rng = rng();

        let nodes: Vec<DeclarationId> = std::iter::repeat_with(|| {
            let mut buff = [0u8; 32];
            rng.fill_bytes(&mut buff);
            DeclarationId(buff)
        })
        .take(100_000)
        .collect();

        let mut assignations: Vec<BTreeSet<DeclarationId>> = std::iter::repeat_with(BTreeSet::new)
            .take(SUBNETWORK_SIZE)
            .collect();

        assignations = HistoryAwareRefill::calculate_subnetwork_assignations(
            &nodes,
            assignations,
            REPLICATION_FACTOR,
            &mut rng,
        );
        assert_assignations(&assignations, &nodes, REPLICATION_FACTOR);

        let mut new_nodes = nodes.clone();
        let mut network_size = new_nodes.len();
        for _ in 0..100 {
            if rng.random_bool(0.5) {
                // shrinking
                network_size = (MIN_NETWORK_SIZE..network_size)
                    .choose_stable(&mut rng)
                    .unwrap();
                new_nodes = shrink_nodes(&new_nodes, network_size).collect();
            } else {
                // growing
                network_size = (network_size..network_size + 1000)
                    .choose_stable(&mut rng)
                    .unwrap();
                new_nodes = expand_nodes(&new_nodes, network_size - new_nodes.len()).collect();
            }
            let third_networks_size = new_nodes.len() / 3;
            mutate_nodes(&mut new_nodes, third_networks_size);
            assignations = HistoryAwareRefill::calculate_subnetwork_assignations(
                &new_nodes,
                assignations,
                REPLICATION_FACTOR,
                &mut rng,
            );
            assert_assignations(&assignations, &new_nodes, REPLICATION_FACTOR);
        }
    }

    #[test]
    fn deterministic_assignations() {
        let assignations: Vec<_> = std::iter::repeat_with(|| {
            let mut rng = SmallRng::seed_from_u64(0);
            test_single_with(SUBNETWORK_SIZE, REPLICATION_FACTOR, 100, &mut rng)
        })
        .take(10)
        .collect();
        assert!(assignations.iter().all(|a| a == &assignations[0]));
    }
}
