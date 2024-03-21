use std::collections::{BTreeMap, HashSet};

use carnot_engine::{LeaderProof, NodeId, View};
use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;
use proptest_state_machine::ReferenceStateMachine;

use crate::fuzz::transition::Transition;
use crate::fuzz::{AggregateQc, Block, Qc, StandardQc, TimeoutQc};

// A reference state machine (RefState) is used to generated state transitions.
// To generate some kinds of transition, we may need to keep historical blocks in RefState.
// Also, RefState can be used to check invariants of the real state machine in some cases.
//
// We should try to design this reference state as simple/intuitive as possible,
// so that we don't need to replicate the logic implemented in carnot-engine.
#[derive(Clone, Debug)]
pub struct RefState {
    pub chain: BTreeMap<View, ViewEntry>,
    pub highest_voted_view: View,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewEntry {
    pub blocks: HashSet<Block>,
    pub timeout_qcs: HashSet<TimeoutQc>,
}

const LEADER_PROOF: LeaderProof = LeaderProof::LeaderId {
    leader_id: NodeId::new([0; 32]),
};
const INITIAL_HIGHEST_VOTED_VIEW: View = View::new(-1);
const SENDER: NodeId = NodeId::new([0; 32]);

impl ReferenceStateMachine for RefState {
    type State = Self;

    type Transition = Transition;

    // Initialize the reference state machine
    fn init_state() -> BoxedStrategy<Self::State> {
        let genesis_block = Block {
            view: View::new(0),
            id: [0; 32],
            parent_qc: Qc::Standard(StandardQc::genesis([0; 32])),
            leader_proof: LEADER_PROOF.clone(),
        };

        Just(RefState {
            chain: BTreeMap::from([(
                genesis_block.view,
                ViewEntry {
                    blocks: HashSet::from([genesis_block]),
                    timeout_qcs: Default::default(),
                },
            )]),
            highest_voted_view: INITIAL_HIGHEST_VOTED_VIEW,
        })
        .boxed()
    }

    // Generate transitions based on the current reference state machine
    fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
        // Instead of using verbose `if` statements here to filter out the types of transitions
        // which cannot be created based on the current reference state,
        // each `state.transition_*` function returns a Nop transition
        // if it cannot generate the promised transition for the current reference state.
        // Both reference and real state machine do nothing for Nop transitions.
        prop_oneof![
            state.transition_receive_safe_block(),
            state.transition_receive_unsafe_block(),
            state.transition_approve_block(),
            state.transition_approve_past_block(),
            state.transition_local_timeout(),
            state.transition_receive_timeout_qc_for_recent_view(),
            state.transition_receive_timeout_qc_for_old_view(),
            state.transition_approve_new_view_with_latest_timeout_qc(),
            state.transition_receive_safe_block_with_aggregated_qc(),
        ]
        .boxed()
    }

    // Check if the transition is valid for a given reference state, before applying the transition
    // If invalid, the transition will be ignored and a new transition will be generated.
    //
    // Also, preconditions are used for shrinking in failure cases.
    // Preconditions check if the transition is still valid after some shrinking is applied.
    // If the transition became invalid for the shrinked state, the shrinking is stopped or
    // is continued to other directions.
    fn preconditions(state: &Self::State, transition: &Self::Transition) -> bool {
        // In most cases, we need to check the same conditions again used to create transitions.
        // This is redundant for success cases, but is necessary for shrinking in failure cases,
        // because some transitions may no longer be valid after some shrinking is applied.
        match transition {
            Transition::Nop => true,
            Transition::ReceiveSafeBlock(block) => {
                state.contains_parent_of(block) && block.view >= state.current_view()
            }
            Transition::ReceiveUnsafeBlock(block) => {
                state.contains_parent_of(block) && block.view < state.current_view()
            }
            Transition::ApproveBlock(block) => state.highest_voted_view < block.view,
            Transition::ApprovePastBlock(block) => state.highest_voted_view >= block.view,
            Transition::LocalTimeout => true,
            Transition::ReceiveTimeoutQcForRecentView(timeout_qc) => {
                timeout_qc.view() >= state.current_view()
            }
            Transition::ReceiveTimeoutQcForOldView(timeout_qc) => {
                timeout_qc.view() < state.current_view()
            }
            Transition::ApproveNewViewWithLatestTimeoutQc(timeout_qc, _) => {
                state.latest_timeout_qcs().contains(timeout_qc)
                    && state.highest_voted_view < RefState::new_view_from(timeout_qc)
            }
        }
    }

    // Apply the given transition on the reference state machine,
    // so that it can be used to generate next transitions.
    fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
        match transition {
            Transition::Nop => {}
            Transition::ReceiveSafeBlock(block) => {
                state
                    .chain
                    .entry(block.view)
                    .or_default()
                    .blocks
                    .insert(block.clone());
            }
            Transition::ReceiveUnsafeBlock(_) => {
                // Nothing to do because we expect the state doesn't change.
            }
            Transition::ApproveBlock(block) => {
                state.highest_voted_view = block.view;
            }
            Transition::ApprovePastBlock(_) => {
                // Nothing to do because we expect the state doesn't change.
            }
            Transition::LocalTimeout => {
                state.highest_voted_view = state.current_view();
            }
            Transition::ReceiveTimeoutQcForRecentView(timeout_qc) => {
                state
                    .chain
                    .entry(timeout_qc.view())
                    .or_default()
                    .timeout_qcs
                    .insert(timeout_qc.clone());
            }
            Transition::ReceiveTimeoutQcForOldView(_) => {
                // Nothing to do because we expect the state doesn't change.
            }
            Transition::ApproveNewViewWithLatestTimeoutQc(timeout_qc, _) => {
                let new_view = RefState::new_view_from(timeout_qc);
                state.chain.entry(new_view).or_default();
                state.highest_voted_view = new_view;
            }
        }

        state
    }
}

impl RefState {
    // Generate a Transition::ReceiveSafeBlock.
    fn transition_receive_safe_block(&self) -> BoxedStrategy<Transition> {
        let parents: Vec<Block> = self
            .chain
            .get(&self.current_view())
            .cloned()
            .unwrap_or_default()
            .blocks
            .into_iter()
            .collect();

        if parents.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            // proptest::sample::select panics if the input is empty
            proptest::sample::select(parents)
                .prop_map(move |parent| -> Transition {
                    Transition::ReceiveSafeBlock(Self::consecutive_block(&parent))
                })
                .boxed()
        }
    }

    // Generate a Transition::ReceiveUnsafeBlock.
    fn transition_receive_unsafe_block(&self) -> BoxedStrategy<Transition> {
        let old_parents = self
            .chain
            .range(..self.current_view().prev())
            .flat_map(|(_view, entry)| entry.blocks.iter().cloned())
            .collect::<Vec<Block>>();

        if old_parents.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            // proptest::sample::select panics if the input is empty
            proptest::sample::select(old_parents)
                .prop_map(move |parent| -> Transition {
                    Transition::ReceiveUnsafeBlock(Self::consecutive_block(&parent))
                })
                .boxed()
        }
    }

    // Generate a Transition::ApproveBlock.
    fn transition_approve_block(&self) -> BoxedStrategy<Transition> {
        let blocks_not_voted = self
            .chain
            .range(self.highest_voted_view.next()..)
            .flat_map(|(_view, entry)| entry.blocks.iter().cloned())
            .collect::<Vec<Block>>();

        if blocks_not_voted.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            // proptest::sample::select panics if the input is empty
            proptest::sample::select(blocks_not_voted)
                .prop_map(Transition::ApproveBlock)
                .boxed()
        }
    }

    // Generate a Transition::ApprovePastBlock.
    fn transition_approve_past_block(&self) -> BoxedStrategy<Transition> {
        let past_blocks = self
            .chain
            .range(INITIAL_HIGHEST_VOTED_VIEW..self.highest_voted_view)
            .flat_map(|(_view, entry)| entry.blocks.iter().cloned())
            .collect::<Vec<Block>>();

        if past_blocks.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            // proptest::sample::select panics if the input is empty
            proptest::sample::select(past_blocks)
                .prop_map(Transition::ApprovePastBlock)
                .boxed()
        }
    }

    // Generate a Transition::LocalTimeout.
    fn transition_local_timeout(&self) -> BoxedStrategy<Transition> {
        Just(Transition::LocalTimeout).boxed()
    }

    // Generate a Transition::ReceiveTimeoutQcForRecentView
    fn transition_receive_timeout_qc_for_recent_view(&self) -> BoxedStrategy<Transition> {
        let current_view: i64 = self.current_view().into();
        let local_high_qc = self.high_qc();
        let delta = 3;

        let blocks_around_local_high_qc = self
            .chain
            .range(local_high_qc.view - View::new(delta)..=local_high_qc.view + View::new(delta)) // including past/future QCs
            .flat_map(|(_, entry)| entry.blocks.iter().cloned())
            .collect::<Vec<Block>>();

        if blocks_around_local_high_qc.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            proptest::sample::select(blocks_around_local_high_qc)
                .prop_flat_map(move |block| {
                    (current_view..=current_view + delta) // including future views
                        .prop_map(move |view| {
                            Transition::ReceiveTimeoutQcForRecentView(TimeoutQc::new(
                                View::new(view),
                                StandardQc {
                                    view: block.view,
                                    id: block.id,
                                },
                                SENDER,
                            ))
                        })
                })
                .boxed()
        }
    }

    // Generate a Transition::ReceiveTimeoutQcForOldView
    fn transition_receive_timeout_qc_for_old_view(&self) -> BoxedStrategy<Transition> {
        let old_view_entries: Vec<(View, ViewEntry)> = self
            .chain
            .range(..self.current_view())
            .filter(|(_, entry)| !entry.is_empty())
            .map(|(&view, entry)| (view, entry.clone()))
            .collect();

        if old_view_entries.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            proptest::sample::select(old_view_entries)
                .prop_map(move |(view, entry)| {
                    Transition::ReceiveTimeoutQcForOldView(TimeoutQc::new(
                        view,
                        entry.high_qc().unwrap(),
                        SENDER,
                    ))
                })
                .boxed()
        }
    }

    // Generate a Transition::ApproveNewViewWithLatestTimeoutQc.
    fn transition_approve_new_view_with_latest_timeout_qc(&self) -> BoxedStrategy<Transition> {
        let latest_timeout_qcs: Vec<TimeoutQc> = self
            .latest_timeout_qcs()
            .iter()
            .filter(|timeout_qc| self.highest_voted_view < RefState::new_view_from(timeout_qc))
            .cloned()
            .collect();

        if latest_timeout_qcs.is_empty() {
            Just(Transition::Nop).boxed()
        } else {
            proptest::sample::select(latest_timeout_qcs)
                .prop_map(move |timeout_qc| {
                    //TODO: set new_views
                    Transition::ApproveNewViewWithLatestTimeoutQc(timeout_qc, HashSet::new())
                })
                .boxed()
        }
    }

    // Generate a Transition::ReceiveSafeBlock, but with AggregatedQc.
    fn transition_receive_safe_block_with_aggregated_qc(&self) -> BoxedStrategy<Transition> {
        //TODO: more randomness
        let current_view = self.current_view();
        let mut id = [0; 32];
        rand::thread_rng().fill_bytes(&mut id);
        Just(Transition::ReceiveSafeBlock(Block {
            view: current_view.next(),
            id,
            parent_qc: Qc::Aggregated(AggregateQc {
                high_qc: self.high_qc(),
                view: current_view,
            }),
            leader_proof: LEADER_PROOF.clone(),
        }))
        .boxed()
    }

    pub fn current_view(&self) -> View {
        let (&last_view, last_entry) = self.chain.last_key_value().unwrap();
        if last_entry.timeout_qcs.is_empty() {
            last_view
        } else {
            let timeout_qc = last_entry.timeout_qcs.iter().next().unwrap();
            RefState::new_view_from(timeout_qc)
        }
    }

    pub fn new_view_from(timeout_qc: &TimeoutQc) -> View {
        timeout_qc.view().next()
    }

    pub fn high_qc(&self) -> StandardQc {
        self.chain
            .values()
            .map(|entry| {
                entry
                    .high_qc()
                    .unwrap_or_else(|| StandardQc::genesis([0; 32]))
            })
            .max_by_key(|qc| qc.view)
            .unwrap_or_else(|| StandardQc::genesis([0; 32]))
    }

    pub fn latest_timeout_qcs(&self) -> Vec<TimeoutQc> {
        let latest_timeout_qc_view_entry = self
            .chain
            .iter()
            .rev()
            .find(|(_, entry)| !entry.timeout_qcs.is_empty());

        match latest_timeout_qc_view_entry {
            Some((_, entry)) => entry
                .timeout_qcs
                .iter()
                .cloned()
                .collect::<Vec<TimeoutQc>>(),
            None => vec![],
        }
    }

    fn contains_parent_of(&self, block: &Block) -> bool {
        self.contains_block(block.parent_qc.block())
    }

    fn contains_block(&self, block_id: [u8; 32]) -> bool {
        self.chain
            .iter()
            .any(|(_, entry)| entry.blocks.iter().any(|block| block.id == block_id))
    }

    fn consecutive_block(parent: &Block) -> Block {
        let mut id = [0; 32];
        rand::thread_rng().fill_bytes(&mut id);
        Block {
            // use rand because we don't want this to be shrinked by proptest
            view: parent.view.next(),
            id,
            parent_qc: Qc::Standard(StandardQc {
                view: parent.view,
                id: parent.id,
            }),
            leader_proof: LEADER_PROOF.clone(),
        }
    }
}

impl ViewEntry {
    fn is_empty(&self) -> bool {
        self.blocks.is_empty() && self.timeout_qcs.is_empty()
    }

    fn high_qc(&self) -> Option<StandardQc> {
        let iter1 = self.blocks.iter().map(|block| block.parent_qc.high_qc());
        let iter2 = self
            .timeout_qcs
            .iter()
            .map(|timeout_qc| timeout_qc.high_qc().clone());
        iter1.chain(iter2).max_by_key(|qc| qc.view)
    }
}
