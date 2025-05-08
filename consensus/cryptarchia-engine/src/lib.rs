#![expect(
    clippy::disallowed_script_idents,
    reason = "The crate `cfg_eval` contains Sinhala script identifiers. \
    Using the `expect` or `allow` macro on top of their usage does not remove the warning"
)]

pub mod config;
pub mod time;

use core::{fmt::Debug, hash::Hash};
use std::collections::{HashMap, HashSet};

pub use config::*;
use thiserror::Error;
pub use time::{Epoch, EpochConfig, Slot};

pub(crate) const LOG_TARGET: &str = "cryptarchia-engine";

#[derive(Clone, Debug)]
pub struct Cryptarchia<Id> {
    local_chain: Branch<Id>,
    branches: Branches<Id>,
    config: Config,
    genesis: Id,
}

impl<Id> PartialEq for Cryptarchia<Id>
where
    Id: Eq + Hash,
{
    fn eq(&self, other: &Self) -> bool {
        self.local_chain == other.local_chain
            && self.branches == other.branches
            && self.config == other.config
            && self.genesis == other.genesis
    }
}

#[derive(Clone, Debug)]
pub struct Branches<Id> {
    branches: HashMap<Id, Branch<Id>>,
    tips: HashSet<Id>,
}

impl<Id> PartialEq for Branches<Id>
where
    Id: Eq + Hash,
{
    fn eq(&self, other: &Self) -> bool {
        self.branches == other.branches && self.tips == other.tips
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Branch<Id> {
    id: Id,
    parent: Id,
    slot: Slot,
    // chain length
    length: u64,
}

impl<Id: Copy> Branch<Id> {
    pub const fn id(&self) -> Id {
        self.id
    }
    pub const fn parent(&self) -> Id {
        self.parent
    }
    pub const fn slot(&self) -> Slot {
        self.slot
    }
    pub const fn length(&self) -> u64 {
        self.length
    }
}

impl<Id> Branches<Id>
where
    Id: Eq + std::hash::Hash + Copy + Debug,
{
    // Creates a new instance from genesis, and assign the genesis block as the
    // parent of itself. This is useful for stopping backwards iterations (i.e.,
    // from most to least recent) without the need for introducing `Option`s.
    pub fn from_genesis(genesis: Id) -> Self {
        let mut branches = HashMap::new();
        branches.insert(
            genesis,
            Branch {
                id: genesis,
                parent: genesis,
                slot: 0.into(),
                length: 0,
            },
        );
        let tips = HashSet::from([genesis]);
        Self { branches, tips }
    }

    /// Create a new [`Branches`] instance with the updated state.
    /// This method adds a [`Branch`] to the new instance without running
    /// validation.
    ///
    /// # Warning
    ///
    /// **This method bypasses safety checks** and can corrupt the state if used
    /// incorrectly.
    /// Only use for recovery, debugging, or other manipulations where the input
    /// is known to be valid.
    ///
    /// # Arguments
    ///
    /// * `header` - The ID of the block to be added.
    /// * `parent` - The ID of the parent block. Due to the nature of the method
    ///   (`unchecked`), the existence of the parent block is not verified.
    /// * `slot` - The slot of the block to be added.
    /// * `chain_length`: The position of the block in the chain.
    #[must_use = "Returns a new instance with the updated state, without modifying the original."]
    fn apply_header_unchecked(
        &self,
        header: Id,
        parent: Id,
        slot: Slot,
        chain_length: u64,
    ) -> Self {
        let mut branches = self.branches.clone();

        let mut tips = self.tips.clone();
        tips.remove(&parent);
        tips.insert(header);

        branches.insert(
            header,
            Branch {
                id: header,
                parent,
                length: chain_length,
                slot,
            },
        );

        Self { branches, tips }
    }

    /// Create a new [`Branches`] instance with the updated state.
    #[must_use = "this returns the result of the operation, without modifying the original"]
    fn apply_header(&self, header: Id, parent: Id, slot: Slot) -> Result<Self, Error<Id>> {
        // Calculating the length here allows us to reuse
        // `Self::apply_header_unchecked`. Could this lead to length difference
        // issues? We are calculating length here but the `self.branches` is
        // cloned in the `Self::apply_header_unchecked` method, which means
        // there's a risk of length being different due to concurrent operations.
        let length = self
            .branches
            .get(&parent)
            .ok_or(Error::ParentMissing(parent))?
            .length
            .checked_add(1)
            .expect("New branch height overflows.");

        Ok(self.apply_header_unchecked(header, parent, slot, length))
    }

    pub fn branches(&self) -> impl Iterator<Item = Branch<Id>> + '_ {
        self.tips.iter().map(|id| self.branches[id])
    }

    // find the lowest common ancestor of two branches
    pub fn lca<'a>(&'a self, mut b1: &'a Branch<Id>, mut b2: &'a Branch<Id>) -> Branch<Id> {
        // first reduce branches to the same length
        while b1.length > b2.length {
            b1 = &self.branches[&b1.parent];
        }

        while b2.length > b1.length {
            b2 = &self.branches[&b2.parent];
        }

        // then walk up the chain until we find the common ancestor
        while b1.id != b2.id {
            b1 = &self.branches[&b1.parent];
            b2 = &self.branches[&b2.parent];
        }

        *b1
    }

    pub fn get(&self, id: &Id) -> Option<&Branch<Id>> {
        self.branches.get(id)
    }

    pub fn get_length_for_header(&self, header_id: &Id) -> Option<u64> {
        self.get(header_id).map(|branch| branch.length)
    }

    // Walk back the chain until the target slot
    fn walk_back_before(&self, branch: &Branch<Id>, slot: Slot) -> Branch<Id> {
        let mut current = branch;
        while current.slot > slot {
            current = &self.branches[&current.parent];
        }
        *current
    }
}

#[derive(Debug, Clone, Error)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub enum Error<Id> {
    #[error("Parent block: {0:?} is not know to this node")]
    ParentMissing(Id),
    #[error("Orphan proof has was not found in the ledger: {0:?}, can't import it")]
    OrphanMissing(Id),
}

impl<Id> Cryptarchia<Id>
where
    Id: Eq + std::hash::Hash + Copy + Debug,
{
    pub fn from_genesis(id: Id, config: Config) -> Self {
        Self {
            branches: Branches::from_genesis(id),
            local_chain: Branch {
                id,
                length: 0,
                parent: id,
                slot: 0.into(),
            },
            config,
            genesis: id,
        }
    }

    /// Create a new [`Cryptarchia`] instance with the updated state.
    /// This method adds a [`Block`] to the new instance without running
    /// validation.
    ///
    /// # Warning
    ///
    /// **This method bypasses safety checks** and can corrupt the state if used
    /// incorrectly.
    /// Only use for recovery, debugging, or other manipulations where the input
    /// is known to be valid.
    ///
    /// # Arguments
    ///
    /// * `header` - The ID of the block to be added.
    /// * `parent` - The ID of the parent block. Due to the nature of the method
    ///   (`unchecked`), the existence of the parent block is not verified.
    /// * `slot` - The slot of the block to be added.
    /// * `chain_length`: The position of the block in the chain.
    #[must_use = "Returns a new instance with the updated state, without modifying the original."]
    pub fn receive_block_unchecked(
        &self,
        header: Id,
        parent: Id,
        slot: Slot,
        chain_length: u64,
    ) -> Self {
        let mut new = self.clone();
        new.branches = new
            .branches
            .apply_header_unchecked(header, parent, slot, chain_length);
        new.local_chain = new.fork_choice();
        new
    }

    /// Create a new [`Cryptarchia`] instance with the updated state.
    #[must_use = "Returns a new instance with the updated state, without modifying the original."]
    pub fn receive_block(&self, id: Id, parent: Id, slot: Slot) -> Result<Self, Error<Id>> {
        let mut new: Self = self.clone();
        new.branches = new.branches.apply_header(id, parent, slot)?;
        new.local_chain = new.fork_choice();
        Ok(new)
    }

    pub fn fork_choice(&self) -> Branch<Id> {
        let k = self.config.security_param.get().into();
        let s = self.config.s();
        Self::maxvalid_bg(self.local_chain, &self.branches, k, s)
    }

    pub const fn tip(&self) -> Id {
        self.local_chain.id
    }

    pub fn non_canonical_forks(&self) -> impl Iterator<Item = Branch<Id>> + '_ {
        self.branches
            .branches()
            .filter(|fork_tip| fork_tip.id != self.tip())
    }

    /// Prune all states strictly deeper than 'depth' with regard to the
    /// current local chain except for states belonging to the local chain.
    ///
    /// For example, if the tip of the canonical chain is at height 10 (i.e., 11
    /// blocks long), calling `self.prune_forks(10)` will remove any forks
    /// stemming from the genesis block, with height `0`, which is the 11th
    /// block in the past.
    ///
    /// This function does not apply any particular logic when evaluating forks
    /// other than the height at which they started diverging from the local
    /// canonical chain.
    ///
    /// It returns the blocks that were part of the pruned forks. The order of
    /// the branches is given by the underlying block storage, but branches
    /// belonging to the same branch will appear from most recent to least
    /// recent in the same order they were applied.
    pub fn prune_forks(&mut self, depth: u64) -> impl Iterator<Item = Branch<Id>> {
        let local_chain = self.local_chain;
        let non_canonical_forks = self.non_canonical_forks();
        let Some(target_height) = local_chain.length.checked_sub(depth) else {
            tracing::info!(
                target: LOG_TARGET,
                "No pruning needed, the canonical chain is not longer than the provided depth. Canonical chain length: {}, provided depth: {}", local_chain.length, depth
            );
            return vec![].into_iter();
        };
        // Calculate LCA between each fork and canonical chain, and consider for pruning
        // if the fork started before the specified `depth`.
        let non_canonical_forks_older_than_depth: Vec<(Branch<Id>, Branch<Id>)> =
            non_canonical_forks
                .filter_map(|fork| {
                    let lca = self.branches.lca(&local_chain, &fork);
                    (lca.length <= target_height).then_some((fork, lca))
                })
                .collect();
        let mut removed_blocks = vec![];
        for (fork, lca) in &non_canonical_forks_older_than_depth {
            removed_blocks.extend(self.prune_fork(fork.id, lca.id));
        }
        removed_blocks.into_iter()
    }

    /// Remove the list of blocks from `tip` to and excluding `up_to`, returning
    /// them in the same order they were encounter while traversing blocks from
    /// `tip` to `up_to`.
    fn prune_fork(&mut self, tip: Id, up_to: Id) -> impl Iterator<Item = Branch<Id>> {
        let tip_removed = self.branches.tips.remove(&tip);
        assert!(
            tip_removed,
            "Provided fork tip should be in the set of tips"
        );
        let mut current_tip = tip;
        let mut removed_blocks = vec![];
        while current_tip != up_to {
            let Some(branch) = self.branches.branches.remove(&current_tip) else {
                // If tip is not in branch set, it means this tip was sharing part of its
                // history with another fork that has already been removed.
                break;
            };
            removed_blocks.push(branch);
            current_tip = branch.parent;
        }
        tracing::debug!(
            target: LOG_TARGET,
            "Pruned branch from {tip:#?} to {up_to:#?}."
        );
        removed_blocks.into_iter()
    }

    pub const fn genesis(&self) -> Id {
        self.genesis
    }

    pub const fn branches(&self) -> &Branches<Id> {
        &self.branches
    }

    // Implementation of the fork choice rule as defined in the Ouroboros Genesis
    // paper.
    //
    //* `k` defines the forking depth of chain we accept without more analysis
    // * `s` defines the length of time (unit of slots) after the fork happened we
    //   will inspect for chain density
    fn maxvalid_bg(local_chain: Branch<Id>, branches: &Branches<Id>, k: u64, s: u64) -> Branch<Id> {
        let mut cmax = local_chain;
        let forks = branches.branches();
        for chain in forks {
            let lowest_common_ancestor = branches.lca(&cmax, &chain);
            let m = cmax
                .length
                .checked_sub(lowest_common_ancestor.length)
                .expect("LCA by definition should be shorter than both of their components.");
            if m <= k {
                // Classic longest chain rule with parameter k
                if cmax.length < chain.length {
                    cmax = chain;
                }
            } else {
                // The chain is forking too much, we need to pay a bit more attention
                // In particular, select the chain that is the densest after the fork
                let density_slot = Slot::from(
                    u64::from(lowest_common_ancestor.slot)
                        .checked_add(s)
                        .expect("Adding LCA slot with the density slot parameter overflows."),
                );
                let cmax_density = branches.walk_back_before(&cmax, density_slot).length;
                let candidate_density = branches.walk_back_before(&chain, density_slot).length;
                if cmax_density < candidate_density {
                    cmax = chain;
                }
            }
        }
        cmax
    }

    pub fn get_security_block_header_id(&self) -> Option<Id> {
        (0..self.config.security_param.get()).try_fold(self.tip(), |header, _| {
            let branch = self.branches.get(&header)?;
            let parent = branch.parent;
            if header == parent {
                // If the header is the genesis block, we arrived at the end of the chain
                None
            } else {
                Some(parent)
            }
        })
    }
}

#[cfg(test)]
pub mod tests {
    use std::{
        collections::HashSet,
        hash::{DefaultHasher, Hash, Hasher as _},
        num::NonZero,
    };

    use super::{Cryptarchia, Slot};
    use crate::Config;

    #[must_use]
    pub const fn config() -> Config {
        Config {
            security_param: NonZero::new(1).unwrap(),
            active_slot_coeff: 1.0,
        }
    }

    fn hash<T: Hash>(t: &T) -> [u8; 32] {
        let mut s = DefaultHasher::new();
        t.hash(&mut s);
        let hash = s.finish();
        let mut res = [0; 32];
        res[..8].copy_from_slice(&hash.to_be_bytes());
        res
    }

    /// Create a canonical chain with the `length` blocks and the provided `c`
    /// config.
    ///
    /// Blocks IDs for blocks other than the genesis are the hash of each block
    /// index, so for a chain of length 10, the sequence of block IDs will be
    /// `[0, hash(1), hash(2), ..., hash(9)]`.
    fn create_canonical_chain(length: NonZero<u64>, c: Option<Config>) -> Cryptarchia<[u8; 32]> {
        let mut engine = Cryptarchia::from_genesis([0; 32], c.unwrap_or_else(config));
        let mut parent = engine.genesis();
        for i in 1..length.get() {
            let new_block = hash(&i);
            engine = engine
                .receive_block(new_block, parent, i.into())
                .expect("test block to be applied successfully.");
            parent = new_block;
        }
        engine
    }

    #[test]
    fn test_fork_choice() {
        // TODO: use cryptarchia
        let mut engine = Cryptarchia::from_genesis([0; 32], config());
        // by setting a low k we trigger the density choice rule, and the shorter chain
        // is denser after the fork
        engine.config.security_param = NonZero::new(10).unwrap();

        let mut parent = engine.genesis();
        for i in 1..50 {
            let new_block = hash(&i);
            engine = engine.receive_block(new_block, parent, i.into()).unwrap();
            parent = new_block;
            println!("{:?}", engine.tip());
        }
        println!("{:?}", engine.tip());
        assert_eq!(engine.tip(), parent);

        let mut long_p = parent;
        let mut short_p = parent;
        // the node sees first the short chain
        for slot in 50..70 {
            let new_block = hash(&format!("short-{slot}"));
            engine = engine
                .receive_block(new_block, short_p, slot.into())
                .unwrap();
            short_p = new_block;
        }

        assert_eq!(engine.tip(), short_p);

        // then it receives a longer chain which is however less dense after the fork
        for slot in 50..70 {
            if slot % 2 == 0 {
                let new_block = hash(&format!("long-{slot}"));
                engine = engine
                    .receive_block(new_block, long_p, slot.into())
                    .unwrap();
                long_p = new_block;
            }
            assert_eq!(engine.tip(), short_p);
        }
        // even if the long chain is much longer, it will never be accepted as it's not
        // dense enough
        for slot in 70..100 {
            let new_block = hash(&format!("long-{slot}"));
            engine = engine
                .receive_block(new_block, long_p, slot.into())
                .unwrap();
            long_p = new_block;
            assert_eq!(engine.tip(), short_p);
        }

        {
            let bs = engine.branches();
            let long_branch = bs.branches().find(|b| b.id == long_p).unwrap();
            let short_branch = bs.branches().find(|b| b.id == short_p).unwrap();

            assert!(long_branch.length > short_branch.length);

            // however, if we set k to the fork length, it will be accepted
            let k = long_branch.length;
            assert_eq!(
                Cryptarchia::maxvalid_bg(short_branch, engine.branches(), k, engine.config.s()).id,
                long_p
            );
        };

        // a longer chain which is equally dense after the fork will be selected as the
        // main tip
        for slot in 50..71 {
            let new_block = hash(&format!("long-dense-{slot}"));
            engine = engine
                .receive_block(new_block, parent, slot.into())
                .unwrap();
            parent = new_block;
        }
        assert_eq!(engine.tip(), parent);
    }

    #[test]
    fn test_getters() {
        let engine = Cryptarchia::from_genesis([0; 32], config());
        let id_0 = engine.genesis();

        // Get branch directly from HashMap
        let branch1 = engine.branches.get(&id_0).expect("branch1 should be there");

        let branches = engine.branches();

        // Get branch using getter
        let branch2 = branches.get(&id_0).expect("branch2 should be there");

        assert_eq!(branch1, branch2);
        assert_eq!(branch1.id(), branch2.id());
        assert_eq!(branch1.parent(), branch2.parent());
        assert_eq!(branch1.slot(), branch2.slot());
        assert_eq!(branch1.length(), branch2.length());

        let slot = Slot::genesis();

        assert_eq!(slot + 10u64, Slot::from(10));

        let id_100 = [100; 32];

        assert!(
            branches.get(&id_100).is_none(),
            "id_100 should not be related to this branch"
        );
    }

    #[test]
    fn test_get_security_block() {
        let mut engine = Cryptarchia::from_genesis([0; 32], config());
        let mut parent_header = engine.genesis();

        assert!(engine.get_security_block_header_id().is_none());

        let headers_size = 10;
        let headers: Vec<_> = (0..headers_size).map(|i| hash(&i)).collect();
        for (slot, header) in headers.iter().enumerate() {
            let current_header = *header;
            engine = engine
                .receive_block(current_header, parent_header, (slot as u64).into())
                .unwrap();
            parent_header = current_header;
        }

        let security_header_position =
            (headers_size - engine.config.security_param.get() - 1) as usize;
        assert_eq!(
            engine.get_security_block_header_id().unwrap(),
            headers[security_header_position]
        );
    }

    #[test]
    fn pruning_too_back_in_time() {
        let chain_pre = create_canonical_chain(50.try_into().unwrap(), None)
            // Add a fork from genesis block
            .receive_block([100; 32], [0; 32], 1.into())
            .expect("test block to be applied successfully.");
        let mut chain = chain_pre.clone();
        assert_eq!(chain.prune_forks(50).count(), 0);
        assert_eq!(chain, chain_pre);
    }

    #[test]
    fn pruning_with_no_fork_old_enough() {
        let chain_pre = create_canonical_chain(50.try_into().unwrap(), None)
            // Add a fork from block 40
            .receive_block([100; 32], hash(&40u64), 41.into())
            .expect("test block to be applied successfully.");
        let mut chain = chain_pre.clone();
        assert_eq!(chain.prune_forks(10).count(), 0);
        assert_eq!(chain, chain_pre);
    }

    #[test]
    fn pruning_with_no_forks() {
        let chain_pre = create_canonical_chain(50.try_into().unwrap(), None);
        let mut chain = chain_pre.clone();
        assert_eq!(chain.prune_forks(50).count(), 0);
        assert_eq!(chain, chain_pre);
        assert_eq!(chain.prune_forks(49).count(), 0);
        assert_eq!(chain, chain_pre);
        assert_eq!(chain.prune_forks(51).count(), 0);
        assert_eq!(chain, chain_pre);
    }

    #[test]
    fn pruning_with_single_fork_old_enough() {
        let chain_pre = create_canonical_chain(50.try_into().unwrap(), None)
            // Add a fork from block 39
            .receive_block([100; 32], hash(&39u64), 40.into())
            .expect("test block to be applied successfully.")
            // Add a fork from block 40
            .receive_block([101; 32], hash(&40u64), 41.into())
            .expect("test block to be applied successfully.");
        let mut chain = chain_pre.clone();
        let pruned_blocks = chain.prune_forks(10);
        assert_eq!(
            pruned_blocks.map(|block| block.id).collect::<HashSet<_>>(),
            [[100; 32]].into()
        );
        assert!(chain_pre.branches.tips.contains(&[100; 32]));
        assert!(chain_pre.branches.branches.contains_key(&[100; 32]));
        assert!(!chain.branches.tips.contains(&[100; 32]));
        assert!(!chain.branches.branches.contains_key(&[100; 32]));
        // Fork at block 40 was not pruned.
        assert!(chain_pre.branches.tips.contains(&[101; 32]));
        assert!(chain_pre.branches.branches.contains_key(&[101; 32]));
        assert!(chain.branches.tips.contains(&[101; 32]));
        assert!(chain.branches.branches.contains_key(&[101; 32]));
    }

    #[test]
    fn pruning_with_multiple_forks_old_enough() {
        let chain_pre = create_canonical_chain(50.try_into().unwrap(), None)
            // Add a first fork from block 39
            .receive_block([100; 32], hash(&39u64), 40.into())
            .expect("test block to be applied successfully.")
            // Add a second fork from block 39
            .receive_block([200; 32], hash(&39u64), 40.into())
            .expect("test block to be applied successfully.")
            // Add a fork from block 40
            .receive_block([101; 32], hash(&40u64), 41.into())
            .expect("test block to be applied successfully.");
        let mut chain = chain_pre.clone();
        let pruned_blocks = chain.prune_forks(10);
        assert_eq!(
            pruned_blocks.map(|block| block.id).collect::<HashSet<_>>(),
            [[100; 32], [200; 32]].into()
        );
        // First fork at block 39 was pruned.
        assert!(chain_pre.branches.tips.contains(&[100; 32]));
        assert!(chain_pre.branches.branches.contains_key(&[100; 32]));
        assert!(!chain.branches.tips.contains(&[100; 32]));
        assert!(!chain.branches.branches.contains_key(&[100; 32]));
        // Second fork at block 39 was pruned.
        assert!(chain_pre.branches.tips.contains(&[200; 32]));
        assert!(chain_pre.branches.branches.contains_key(&[200; 32]));
        assert!(!chain.branches.tips.contains(&[200; 32]));
        assert!(!chain.branches.branches.contains_key(&[200; 32]));
        // Fork at block 40 was not pruned.
        assert!(chain_pre.branches.tips.contains(&[101; 32]));
        assert!(chain_pre.branches.branches.contains_key(&[101; 32]));
        assert!(chain.branches.tips.contains(&[101; 32]));
        assert!(chain.branches.branches.contains_key(&[101; 32]));
    }

    #[test]
    fn pruning_fork_with_multiple_tips() {
        let chain_pre = create_canonical_chain(50.try_into().unwrap(), None)
            // Add a 2-block fork from block 39
            .receive_block([100; 32], hash(&39u64), 40.into())
            .expect("test block to be applied successfully.")
            .receive_block([101; 32], [100; 32], 41.into())
            .expect("test block to be applied successfully.")
            // Add a second fork from the first divergent fork block, so that the fork has two tips
            .receive_block([200; 32], [100; 32], 42.into())
            .expect("test block to be applied successfully.");
        let mut chain = chain_pre.clone();
        let pruned_blocks = chain.prune_forks(10);
        assert_eq!(
            pruned_blocks.map(|block| block.id).collect::<HashSet<_>>(),
            [[100; 32], [101; 32], [200; 32]].into()
        );
        // First fork was pruned entirely (both tips were removed).
        assert!(chain_pre.branches.tips.contains(&[101; 32]));
        assert!(chain_pre.branches.branches.contains_key(&[100; 32]));
        assert!(chain_pre.branches.branches.contains_key(&[101; 32]));
        assert!(!chain.branches.tips.contains(&[101; 32]));
        assert!(!chain.branches.branches.contains_key(&[100; 32]));
        assert!(!chain.branches.branches.contains_key(&[101; 32]));
        // Second fork was pruned.
        assert!(chain_pre.branches.tips.contains(&[200; 32]));
        assert!(chain_pre.branches.branches.contains_key(&[200; 32]));
        assert!(!chain.branches.tips.contains(&[200; 32]));
        assert!(!chain.branches.branches.contains_key(&[200; 32]));
    }
}
