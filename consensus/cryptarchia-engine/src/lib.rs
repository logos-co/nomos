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

pub(crate) const LOG_TARGET: &str = "cryptarchia::engine";

#[derive(Clone, Debug, Copy)]
pub struct Boostrapping;
#[derive(Clone, Debug, Copy)]
pub struct Online;

pub trait CryptarchiaState: Copy + Debug {
    fn fork_choice<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Branch<Id>
    where
        Id: Eq + Hash + Copy;
    fn lib<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Branch<Id>
    where
        Id: Eq + Hash + Copy;
}

impl CryptarchiaState for Boostrapping {
    fn fork_choice<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Branch<Id>
    where
        Id: Eq + Hash + Copy,
    {
        maxvalid_bg(cryptarchia)
    }

    fn lib<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Branch<Id>
    where
        Id: Eq + Hash + Copy,
    {
        *cryptarchia.lib_branch()
    }
}

impl CryptarchiaState for Online {
    fn fork_choice<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Branch<Id>
    where
        Id: Eq + Hash + Copy,
    {
        maxvalid_mc(cryptarchia)
    }

    fn lib<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Branch<Id>
    where
        Id: Eq + Hash + Copy,
    {
        cryptarchia.nth_ancestor(
            &cryptarchia.local_chain,
            cryptarchia.config.security_param.get().into(),
        )
    }
}

// Implementation of the fork choice rule as defined in the Ouroboros Genesis
// paper k defines the forking depth of chain we accept without more
// analysis s defines the length of time (unit of slots) after the fork
// happened we will inspect for chain density
fn maxvalid_bg<Id>(cryptarchia: &Cryptarchia<Id, Boostrapping>) -> Branch<Id>
where
    Id: Eq + Hash + Copy,
{
    let k = cryptarchia.config.security_param.get().into();
    let s = cryptarchia.config.s();

    let mut cmax = cryptarchia.local_chain;
    let forks = cryptarchia.branches();
    for chain in forks {
        let lowest_common_ancestor = cryptarchia.lca(&cmax, &chain).expect("LCA must be found");
        let m = cmax.length - lowest_common_ancestor.length;
        if m <= k {
            // Classic longest chain rule with parameter k
            if cmax.length < chain.length {
                cmax = chain;
            }
        } else {
            // The chain is forking too much, we need to pay a bit more attention
            // In particular, select the chain that is the densest after the fork
            let density_slot = Slot::from(u64::from(lowest_common_ancestor.slot) + s);
            let cmax_density = cryptarchia
                .walk_back_before(&cmax, density_slot)
                .expect("WalkBack must succeed")
                .length;
            let candidate_density = cryptarchia
                .walk_back_before(&chain, density_slot)
                .expect("WalkBack must succeed")
                .length;
            if cmax_density < candidate_density {
                cmax = chain;
            }
        }
    }
    cmax
}

// Implementation of the fork choice rule as defined in the Ouroboros Praos
// paper k defines the forking depth of chain we can accept
fn maxvalid_mc<Id>(cryptarchia: &Cryptarchia<Id, Online>) -> Branch<Id>
where
    Id: Eq + Hash + Copy,
{
    let k = cryptarchia.config.security_param.get().into();

    let mut cmax = cryptarchia.local_chain;
    let forks = cryptarchia.branches();
    for chain in forks {
        let lowest_common_ancestor = cryptarchia.lca(&cmax, &chain).expect("LCA must be found");
        let m = cmax.length - lowest_common_ancestor.length;
        if m <= k && cmax.length < chain.length {
            // Classic longest chain rule with parameter k
            cmax = chain;
        }
    }
    cmax
}

/// A cryptarchia engine that manages the block tree.
///
/// Whenever a new block is received, it performs the fork choice rule
/// and update the LIB if needed (depending on [`CryptarchiaState`]).
///
/// This guarantees that only blocks descending from the LIB are accepted,
/// by proactively pruning forks that are diverged before the LIB.
#[derive(Clone, Debug)]
pub struct Cryptarchia<Id, State: ?Sized> {
    /// Blocks that are ancestors of the [`Self::lib`].
    /// It is guaranteed that these blocks constitute a single, fork-less chain.
    branches_before_lib: HashMap<Id, Branch<Id>>,
    /// Blocks descending from the [`Self::lib`].
    /// These blocks may form multiple forks.
    branches_from_lib: HashMap<Id, Branch<Id>>,
    /// Tips of the branches descending from the [`Self::lib`],
    /// including the local chain tip.
    /// It is guaranteed that these tips are in [`Self::branches_from_lib`].
    tips_from_lib: HashSet<Id>,
    /// The tip of the local honest chain.
    local_chain: Branch<Id>,
    /// The latest immutable block (LIB) in the local honest chain.
    lib: Id,
    /// Configuration
    config: Config,
    // Just a marker to indicate whether the node is bootstrapping or online.
    // Does not actually end up in memory.
    _state: std::marker::PhantomData<State>,
}

impl<Id, State> PartialEq for Cryptarchia<Id, State>
where
    Id: Eq + Hash,
{
    fn eq(&self, other: &Self) -> bool {
        self.branches_before_lib == other.branches_before_lib
            && self.branches_from_lib == other.branches_from_lib
            && self.tips_from_lib == other.tips_from_lib
            && self.local_chain == other.local_chain
            && self.lib == other.lib
            && self.config == other.config
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

#[derive(Debug, Clone, Error)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub enum Error<Id> {
    #[error("Parent block: {0:?} is not know to this node")]
    ParentMissing(Id),
    #[error("Orphan proof has was not found in the ledger: {0:?}, can't import it")]
    OrphanMissing(Id),
    #[error("Attempting to fork immutable history at {0:?}")]
    ImmutableFork(Id),
    #[error("Invalid slot for block {0:?}, parent slot is greater than child slot")]
    InvalidSlot(Id),
}

/// Information about a fork's divergence from the canonical branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkDivergenceInfo<Id> {
    /// The tip of the diverging fork.
    pub tip: Branch<Id>,
    /// The LCA (lowest common ancestor) of the fork and the local canonical
    /// chain.
    pub lca: Branch<Id>,
}

impl<Id, State> Cryptarchia<Id, State>
where
    Id: Eq + Hash + Copy,
    State: CryptarchiaState + Copy,
{
    pub fn from_lib(lib: Id, config: Config) -> Self {
        let lib_branch = Branch {
            id: lib,
            parent: lib,
            slot: 0.into(),
            length: 0,
        };
        Self {
            branches_before_lib: HashMap::new(),
            branches_from_lib: HashMap::from([(lib, lib_branch)]),
            tips_from_lib: HashSet::from([lib]),
            local_chain: lib_branch,
            lib,
            config,
            _state: std::marker::PhantomData,
        }
    }

    /// Create a new [`Cryptarchia`] instance with the updated state
    /// after applying the given block.
    ///
    /// Also returns [`PrunedBlocks`] if the LIB is updated and forks
    /// that diverged before the new LIB are pruned.
    /// Otherwise, an empty [`PrunedBlocks`] is returned.
    #[must_use = "Returns a new instance with the updated state, without modifying the original."]
    pub fn receive_block(
        &self,
        id: Id,
        parent: Id,
        slot: Slot,
    ) -> Result<(Self, PrunedBlocks<Id>), Error<Id>> {
        // The parent must be searched from the [`branches_from_lib`]
        // to ensure that the new block is a descendant of the LIB.
        //
        // If the parent is not found, it means two cases:
        // 1. The block is part of forks diverged from deeper than LIB.
        // 2. Not the case 1, but the parent is just not known to the node.
        //
        // Checking whether it's case 1 is not cheap if there are missing ancestors.
        // It is confident that it is case 1 if the parent is found in
        // `branches_before_lib`.
        // `ImmutableFork` error is returned in this case, indicating that
        // the block must be rejected.
        //
        // Otherwise, it's either case 1 or 2. Then, it returns `ParentMissing` error,
        // which is expected to be handled by the caller.
        let parent_branch = self.branches_from_lib.get(&parent).ok_or_else(|| {
            if self.branches_before_lib.contains_key(&parent) {
                Error::ImmutableFork(parent)
            } else {
                Error::ParentMissing(parent)
            }
        })?;

        if parent_branch.slot > slot {
            return Err(Error::InvalidSlot(parent));
        }

        let length = parent_branch
            .length
            .checked_add(1)
            .expect("New branch height overflows.");

        // Apply the new block.
        let mut new = self.clone();
        new.tips_from_lib.remove(&parent);
        new.tips_from_lib.insert(id);
        new.branches_from_lib.insert(
            id,
            Branch {
                id,
                parent,
                slot,
                length,
            },
        );
        // Perform the fork choice and update the LIB if needed.
        new.local_chain = new.fork_choice();
        let pruned_blocks = new.update_lib();
        Ok((new, pruned_blocks))
    }

    /// Attempts to update the LIB.
    /// Whether the LIB is actually updated or not depends on the
    /// current [`CryptarchiaState`].
    ///
    /// If the LIB is updated, forks that diverged before the new LIB
    /// are pruned, and the blocks of the pruned forks are returned.
    /// as [`PrunedBlocks`].
    /// Otherwise, an empty [`PrunedBlocks`] is returned.
    fn update_lib(&mut self) -> PrunedBlocks<Id> {
        let new_lib = <State as CryptarchiaState>::lib(&*self);
        if new_lib.id() == self.lib {
            // No change in LIB, nothing to prune.
            return PrunedBlocks::new();
        }

        let lib_depth_from_tip = self
            .tip_branch()
            .length()
            .checked_sub(new_lib.length())
            .expect("Local chain height must be >= LIB height");

        let pruned_blocks = self.prune_forks(lib_depth_from_tip).collect();

        // Move all parents of the new LIB
        // from `branches_from_lib` to `branches_before_lib`.
        let mut parent = new_lib.parent;
        while let Some(parent_branch) = self.branches_from_lib.remove(&parent) {
            self.branches_before_lib.insert(parent, parent_branch);
            parent = parent_branch.parent;
        }

        self.lib = new_lib.id();
        pruned_blocks
    }

    fn fork_choice(&self) -> Branch<Id> {
        <State as CryptarchiaState>::fork_choice(self)
    }

    pub const fn tip(&self) -> Id {
        self.local_chain.id
    }

    pub const fn tip_branch(&self) -> &Branch<Id> {
        &self.local_chain
    }

    /// Prune all blocks that are included in forks that diverged before
    /// the `max_div_depth`-th block from the current local chain tip.
    /// It returns the block IDs that were part of the pruned forks.
    ///
    /// For example,
    /// Given a block tree:
    ///               b6
    ///             /
    /// G - b1 - b2 - b3 - b4 - b5 == local chain tip
    ///                  \
    ///                    b7
    /// Calling `prune_forks(2)` will remove `b6` because it is diverged from
    /// `b2`, which is deeper than the 2nd block `b3` from the local chain tip.
    /// The `b7` is not removed since it is diverged from `b3`.
    fn prune_forks(&mut self, max_div_depth: u64) -> impl Iterator<Item = Id> + '_ {
        #[expect(
            clippy::needless_collect,
            reason = "We need to collect since we cannot borrow both immutably (in `self.prunable_forks`) and mutably (in `self.prune_fork`) at the same time."
        )]
        // Collect prunable forks first to avoid borrowing issues
        let forks: Vec<_> = self.prunable_forks(max_div_depth).collect();
        forks
            .into_iter()
            .flat_map(move |prunable_fork_info| self.prune_fork(&prunable_fork_info))
    }

    /// Get an iterator over the prunable forks that diverged before
    /// the `max_div_depth`-th block from the current local chain tip.
    fn prunable_forks(
        &self,
        max_div_depth: u64,
    ) -> impl Iterator<Item = ForkDivergenceInfo<Id>> + '_ {
        let local_chain = self.local_chain;
        let Some(deepest_div_block) = local_chain.length.checked_sub(max_div_depth) else {
            tracing::debug!(
                target: LOG_TARGET,
                "No prunable fork, the canonical chain is not longer than the provided depth. Canonical chain length: {}, provided max_div_depth: {}", local_chain.length, max_div_depth
            );
            return Box::new(core::iter::empty())
                as Box<dyn Iterator<Item = ForkDivergenceInfo<Id>>>;
        };
        Box::new(self.non_canonical_forks().filter_map(move |fork| {
            // We calculate LCA once and store it in `ForkInfo` so it can be consumed
            // elsewhere without the need to re-calculate it.
            let lca = self
                .lca(&local_chain, &fork)
                .expect("LCA must exist between fork and local chain");
            // If the fork is diverged deeper than `deepest_div_block`, it's prunable.
            (lca.length < deepest_div_block).then_some(ForkDivergenceInfo { tip: fork, lca })
        }))
    }

    /// Returns all the forks that are not part of the local canonical chain.
    ///
    /// The result contains both prunable and non prunable forks.
    pub fn non_canonical_forks(&self) -> impl Iterator<Item = Branch<Id>> + '_ {
        self.branches().filter(|fork_tip| fork_tip.id != self.tip())
    }

    /// Remove all blocks of a fork from `tip` to `lca`, excluding `lca`.
    fn prune_fork(&mut self, &ForkDivergenceInfo { lca, tip }: &ForkDivergenceInfo<Id>) -> Vec<Id> {
        let tip_removed = self.tips_from_lib.remove(&tip.id);
        if !tip_removed {
            tracing::error!(target: LOG_TARGET, "Fork tip not found in the set of tips.");
        }

        let mut current_tip = tip.id;
        let mut removed_blocks = vec![];
        while current_tip != lca.id {
            let Some(branch) = self.branches_from_lib.remove(&current_tip) else {
                // If tip is not in branch set, it means this tip was sharing part of its
                // history with another fork that has already been removed.
                break;
            };
            removed_blocks.push(branch.id);
            current_tip = branch.parent;
        }
        tracing::debug!(
            target: LOG_TARGET,
            "Pruned {} blocks.", removed_blocks.len()
        );
        removed_blocks
    }

    fn branches(&self) -> impl Iterator<Item = Branch<Id>> + '_ {
        self.tips_from_lib.iter().map(|id| {
            *self
                .branches_from_lib
                .get(id)
                .expect("Tip must be present in branches_from_lib.")
        })
    }

    pub fn get(&self, id: &Id) -> Option<&Branch<Id>> {
        self.branches_from_lib
            .get(id)
            .or_else(|| self.branches_before_lib.get(id))
    }

    /// Get the latest immutable block (LIB) in the chain. No re-orgs past this
    /// point are allowed.
    pub const fn lib(&self) -> Id {
        self.lib
    }

    pub fn lib_branch(&self) -> &Branch<Id> {
        self.branches_from_lib
            .get(&self.lib)
            .expect("LIB must be present in branches_from_lib.")
    }

    /// Find the lowest common ancestor of two branches.
    /// If an unknown block is encountered during the search, it returns `None`.
    pub fn lca<'a>(&'a self, mut b1: &'a Branch<Id>, mut b2: &'a Branch<Id>) -> Option<Branch<Id>> {
        // first reduce branches to the same length
        while b1.length > b2.length {
            b1 = self.get(&b1.parent)?;
        }

        while b2.length > b1.length {
            b2 = self.get(&b2.parent)?;
        }

        // then walk up the chain until we find the common ancestor
        while b1.id != b2.id {
            b1 = self.get(&b1.parent)?;
            b2 = self.get(&b2.parent)?;
        }

        Some(*b1)
    }

    /// Walk back the chain until the target slot.
    /// [`None`] is returned if an unknown block is encountered.
    fn walk_back_before(&self, branch: &Branch<Id>, slot: Slot) -> Option<Branch<Id>> {
        let mut current = branch;
        while current.slot > slot {
            current = self.get(&current.parent)?;
        }
        Some(*current)
    }

    /// Returns the min(n, A)-th ancestor of the provided block, where A is the
    /// number of ancestors of this block.
    fn nth_ancestor(&self, branch: &Branch<Id>, mut n: u64) -> Branch<Id> {
        let mut current = branch;
        while n > 0 {
            n -= 1;
            if let Some(parent) = self.get(&current.parent) {
                current = parent;
            } else {
                return *current;
            }
        }
        *current
    }
}

impl<Id> Cryptarchia<Id, Boostrapping>
where
    Id: Eq + Hash + Copy + Debug,
{
    /// Signal transitioning to the online state.
    pub fn online(self) -> (Cryptarchia<Id, Online>, PrunedBlocks<Id>) {
        let mut new = Cryptarchia {
            branches_before_lib: self.branches_before_lib,
            branches_from_lib: self.branches_from_lib,
            tips_from_lib: self.tips_from_lib,
            local_chain: self.local_chain,
            lib: self.lib,
            config: self.config,
            _state: std::marker::PhantomData,
        };
        // Update the LIB to the current local chain's tip
        let pruned_blocks = new.update_lib();
        (new, pruned_blocks)
    }
}

pub type PrunedBlocks<Id> = HashSet<Id>;

#[cfg(test)]
pub mod tests {
    use std::{
        hash::{DefaultHasher, Hash, Hasher as _},
        num::NonZero,
    };

    use super::{maxvalid_bg, Boostrapping, Cryptarchia, Error, Slot};
    use crate::{Config, Online};

    #[must_use]
    pub const fn config() -> Config {
        config_with(1)
    }

    #[must_use]
    pub const fn config_with(security_param: u32) -> Config {
        Config {
            security_param: NonZero::new(security_param).unwrap(),
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
    fn create_canonical_chain(
        length: NonZero<u64>,
        c: Option<Config>,
    ) -> Cryptarchia<[u8; 32], Boostrapping> {
        let mut engine = Cryptarchia::from_lib([0; 32], c.unwrap_or_else(config));
        let mut parent = engine.lib();
        for i in 1..length.get() {
            let new_block = hash(&i);
            engine = engine
                .receive_block(new_block, parent, i.into())
                .expect("test block to be applied successfully.")
                .0;
            parent = new_block;
        }
        engine
    }

    #[test]
    fn test_slot_increasing() {
        // parent
        // └── child

        let engine = Cryptarchia::<_, Boostrapping>::from_lib([0; 32], config());
        let parent = [1; 32];
        let child = [2; 32];

        let engine = engine.receive_block(parent, [0; 32], 2.into()).unwrap().0;
        assert!(matches!(
            engine.receive_block(child, parent, 1.into()),
            Err(Error::InvalidSlot(_))
        ));
    }

    #[test]
    fn test_immutable_fork() {
        // b1
        // └── LIB
        // |    └── b2
        // └── b3

        let mut engine = Cryptarchia::<_, Online>::from_lib([0; 32], config_with(1));
        let b1 = [1; 32];
        let lib = [2; 32];
        let b2 = [3; 32];
        let b3 = [4; 32];
        engine = engine.receive_block(b1, [0; 32], 1.into()).unwrap().0;
        engine = engine.receive_block(lib, b1, 2.into()).unwrap().0;
        engine = engine.receive_block(b2, lib, 3.into()).unwrap().0;
        assert!(matches!(
            engine.receive_block(b3, b1, 4.into()),
            Err(Error::ImmutableFork(_))
        ));
    }

    #[test]
    fn test_fork_choice() {
        // TODO: use cryptarchia
        let mut engine = <Cryptarchia<_, Boostrapping>>::from_lib([0; 32], config());
        // by setting a low k we trigger the density choice rule, and the shorter chain
        // is denser after the fork
        engine.config.security_param = NonZero::new(10).unwrap();

        let mut parent = engine.lib();
        for i in 1..50 {
            let new_block = hash(&i);
            engine = engine.receive_block(new_block, parent, i.into()).unwrap().0;
            parent = new_block;
        }
        assert_eq!(engine.tip(), parent);

        let mut long_p = parent;
        let mut short_p = parent;
        // the node sees first the short chain
        for slot in 50..70 {
            let new_block = hash(&format!("short-{slot}"));
            engine = engine
                .receive_block(new_block, short_p, slot.into())
                .unwrap()
                .0;
            short_p = new_block;
        }

        assert_eq!(engine.tip(), short_p);

        // then it receives a longer chain which is however less dense after the fork
        for slot in 50..70 {
            if slot % 2 == 0 {
                let new_block = hash(&format!("long-{slot}"));
                engine = engine
                    .receive_block(new_block, long_p, slot.into())
                    .unwrap()
                    .0;
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
                .unwrap()
                .0;
            long_p = new_block;
            assert_eq!(engine.tip(), short_p);
        }

        let long_branch = engine.get(&long_p).unwrap();
        let short_branch = engine.get(&short_p).unwrap();

        // however, if we set k to the fork length, it will be accepted
        let mut new_engine = engine.clone();
        new_engine.config.security_param =
            NonZero::new(long_branch.length.try_into().unwrap()).unwrap();
        new_engine.local_chain = *short_branch;
        assert_eq!(maxvalid_bg(&new_engine).id, long_p);

        // a longer chain which is equally dense after the fork will be selected as the
        // main tip
        for slot in 50..71 {
            let new_block = hash(&format!("long-dense-{slot}"));
            engine = engine
                .receive_block(new_block, parent, slot.into())
                .unwrap()
                .0;
            parent = new_block;
        }
        assert_eq!(engine.tip(), parent);
    }

    #[test]
    fn test_getters() {
        let engine = <Cryptarchia<_, Boostrapping>>::from_lib([0; 32], config());
        let id_0 = engine.lib();

        // Get branch directly from HashMap
        let branch1 = engine.get(&id_0).expect("branch1 should be there");

        // Get branch using getter
        let branch2 = engine.get(&id_0).expect("branch2 should be there");

        assert_eq!(branch1, branch2);
        assert_eq!(branch1.id(), branch2.id());
        assert_eq!(branch1.parent(), branch2.parent());
        assert_eq!(branch1.slot(), branch2.slot());
        assert_eq!(branch1.length(), branch2.length());

        let slot = Slot::genesis();

        assert_eq!(slot + 10u64, Slot::from(10));

        let id_100 = [100; 32];

        assert!(
            engine.get(&id_100).is_none(),
            "id_100 should not be related to this branch"
        );
    }

    // It tests that nothing is pruned when the pruning depth is greater than the
    // canonical chain length.
    #[test]
    fn pruning_too_back_in_time() {
        // Create a chain with 50 blocks (0 to 49) with k=51.
        let (cryptarchia, pruned_blocks) =
            create_canonical_chain(50.try_into().unwrap(), Some(config_with(51)))
                // Add a fork from genesis block
                .receive_block([100; 32], [0; 32], 1.into())
                .expect("test block to be applied successfully.");
        // No block is pruned during Boostrapping.
        assert!(pruned_blocks.is_empty());

        // Switch to Online to update LIB and trigger pruning.
        let (cryptarchia, pruned_blocks) = cryptarchia.online();

        // But, no block is pruned because `security_param` is
        // greater than local chain length.
        assert!(pruned_blocks.is_empty());
        assert!(cryptarchia.tips_from_lib.contains(&[100; 32]));
        assert!(cryptarchia.get(&[100; 32]).is_some());
    }

    #[test]
    fn pruning_with_no_fork_old_enough() {
        // Create a chain with 50 blocks (0 to 49) with k=10.
        let (cryptarchia, pruned_blocks) =
            create_canonical_chain(50.try_into().unwrap(), Some(config_with(10)))
                // Add a fork from block 40, which is shallower than LIB
                .receive_block([100; 32], hash(&40u64), 41.into())
                .expect("test block to be applied successfully.");
        // No block is pruned during Boostrapping.
        assert!(pruned_blocks.is_empty());

        // Switch to Online to update LIB and trigger pruning.
        let (cryptarchia, pruned_blocks) = cryptarchia.online();

        // But, no block is pruned.
        assert!(pruned_blocks.is_empty());
        assert!(cryptarchia.tips_from_lib.contains(&[100; 32]));
        assert!(cryptarchia.get(&[100; 32]).is_some());
    }

    #[test]
    fn pruning_with_no_forks() {
        let (_, pruned_blocks) =
            create_canonical_chain(50.try_into().unwrap(), Some(config_with(1))).online();
        assert!(pruned_blocks.is_empty());
    }

    #[test]
    fn pruning_with_single_fork_old_enough() {
        // Create a chain with 50 blocks (0 to 49) with k=10.
        let (cryptarchia, pruned_blocks) =
            create_canonical_chain(50.try_into().unwrap(), Some(config_with(10)))
                // Add a fork from block 38, deeper than LIB
                .receive_block([100; 32], hash(&38u64), 39.into())
                .expect("test block to be applied successfully.")
                .0
                // Add a fork from block 39 (LIB)
                .receive_block([101; 32], hash(&39u64), 40.into())
                .expect("test block to be applied successfully.")
                .0
                // Add a fork from block 40, shallower than LIB
                .receive_block([102; 32], hash(&40u64), 41.into())
                .expect("test block to be applied successfully.");
        // No block is pruned during Boostrapping.
        assert!(pruned_blocks.is_empty());

        // Switch to Online to update LIB and trigger pruning.
        let (cryptarchia, pruned_blocks) = cryptarchia.online();

        // A fork from block 38 is pruned.
        assert_eq!(pruned_blocks, [[100; 32]].into());
        assert!(!cryptarchia.tips_from_lib.contains(&[100; 32]));
        assert!(cryptarchia.get(&[100; 32]).is_none());
        // Fork at block 39 was not pruned because it is diverged
        // at the 10th block (LIB) from the local chain tip.
        assert!(cryptarchia.tips_from_lib.contains(&[101; 32]));
        assert!(cryptarchia.get(&[101; 32]).is_some());
        // Fork at block 40 was not pruned because it is diverged
        // after the 10th block (LIB) from the local chain tip.
        assert!(cryptarchia.tips_from_lib.contains(&[102; 32]));
        assert!(cryptarchia.get(&[102; 32]).is_some());
    }

    #[test]
    fn pruning_with_multiple_forks_old_enough() {
        // Create a chain with 50 blocks (0 to 49) with k=10.
        let (cryptarchia, pruned_blocks) =
            create_canonical_chain(50.try_into().unwrap(), Some(config_with(10)))
                // Add a first fork from block 38
                .receive_block([100; 32], hash(&38u64), 39.into())
                .expect("test block to be applied successfully.")
                .0
                // Add a second fork from block 38
                .receive_block([200; 32], hash(&38u64), 39.into())
                .expect("test block to be applied successfully.")
                .0
                // Add a fork from block 39
                .receive_block([101; 32], hash(&39u64), 40.into())
                .expect("test block to be applied successfully.");
        // No block is pruned during Boostrapping.
        assert!(pruned_blocks.is_empty());

        // Switch to Online to update LIB and trigger pruning.
        let (cryptarchia, pruned_blocks) = cryptarchia.online();

        assert_eq!(pruned_blocks, [[100; 32], [200; 32]].into());
        // First fork at block 38 was pruned.
        assert!(!cryptarchia.tips_from_lib.contains(&[100; 32]));
        assert!(cryptarchia.get(&[100; 32]).is_none());
        // Second fork at block 38 was pruned.
        assert!(!cryptarchia.tips_from_lib.contains(&[200; 32]));
        assert!(cryptarchia.get(&[200; 32]).is_none());
        // Fork at block 40 was not pruned.
        assert!(cryptarchia.tips_from_lib.contains(&[101; 32]));
        assert!(cryptarchia.get(&[101; 32]).is_some());
    }

    #[test]
    fn pruning_fork_with_multiple_tips() {
        // Create a chain with 50 blocks (0 to 49) with k=10.
        let (cryptarchia, pruned_blocks) =
            create_canonical_chain(50.try_into().unwrap(), Some(config_with(10)))
                // Add a 2-block fork from block 38
                .receive_block([100; 32], hash(&38u64), 39.into())
                .expect("test block to be applied successfully.")
                .0
                .receive_block([101; 32], [100; 32], 40.into())
                .expect("test block to be applied successfully.")
                .0
                // Add a second fork from the first divergent fork block, so that the fork has two
                // tips
                .receive_block([200; 32], [100; 32], 41.into())
                .expect("test block to be applied successfully.");
        // No block is pruned during Boostrapping.
        assert!(pruned_blocks.is_empty());

        // Switch to Online to update LIB and trigger pruning.
        let (cryptarchia, pruned_blocks) = cryptarchia.online();

        assert_eq!(pruned_blocks, [[100; 32], [101; 32], [200; 32]].into());
        // First fork was pruned entirely (both tips were removed).
        assert!(!cryptarchia.tips_from_lib.contains(&[101; 32]));
        assert!(cryptarchia.get(&[100; 32]).is_none());
        assert!(cryptarchia.get(&[101; 32]).is_none());
        // Second fork was pruned.
        assert!(!cryptarchia.tips_from_lib.contains(&[200; 32]));
        assert!(cryptarchia.get(&[200; 32]).is_none());
    }

    #[test]
    fn pruning_forks_when_receive_block() {
        // Create an Online chain with 10 blocks (0 to 9) with k=2.
        let (cryptarchia, pruned_blocks) =
            create_canonical_chain(10.try_into().unwrap(), Some(config_with(2))).online();
        // No block is pruned since no fork existed.
        assert!(pruned_blocks.is_empty());

        // Add a fork at the LIB
        let (cryptarchia, pruned_blocks) = cryptarchia
            .receive_block(
                [100; 32],
                cryptarchia.lib(),
                cryptarchia.lib_branch().slot + 1,
            )
            .expect("test block to be applied successfully.");
        // No block is pruned since LIB was not updated.
        assert!(pruned_blocks.is_empty());
        assert!(cryptarchia.tips_from_lib.contains(&[100; 32]));
        assert!(cryptarchia.get(&[100; 32]).is_some());

        // Add a fork after than LIB
        let (cryptarchia, pruned_blocks) = cryptarchia
            .receive_block(
                [101; 32],
                cryptarchia.tip_branch().parent,
                cryptarchia.tip_branch().slot,
            )
            .expect("test block to be applied successfully.");
        // No block is pruned since LIB was not updated.
        assert!(pruned_blocks.is_empty());
        assert!(cryptarchia.tips_from_lib.contains(&[101; 32]));
        assert!(cryptarchia.get(&[101; 32]).is_some());

        // Add a block to the tip to update the LIB.
        let (cryptarchia, pruned_blocks) = cryptarchia
            .receive_block(
                [102; 32],
                cryptarchia.tip(),
                cryptarchia.tip_branch().slot + 1,
            )
            .expect("test block to be applied successfully.");
        // One fork is pruned since LIB is updated.
        assert_eq!(pruned_blocks, [[100; 32]].into());
        assert!(!cryptarchia.tips_from_lib.contains(&[100; 32]));
        assert!(cryptarchia.get(&[100; 32]).is_none());
        assert!(cryptarchia.tips_from_lib.contains(&[101; 32]));
        assert!(cryptarchia.get(&[101; 32]).is_some());
        assert!(cryptarchia.tips_from_lib.contains(&[102; 32]));
        assert!(cryptarchia.get(&[102; 32]).is_some());
    }
}
