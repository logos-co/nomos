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

pub trait CryptarchiaState: Copy + std::fmt::Debug {
    fn fork_choice<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Branch<Id>
    where
        Id: Eq + std::hash::Hash + Copy;
    fn lib<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Id
    where
        Id: Eq + std::hash::Hash + Copy;
}

impl CryptarchiaState for Boostrapping {
    fn fork_choice<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Branch<Id>
    where
        Id: Eq + std::hash::Hash + Copy,
    {
        let k = cryptarchia.config.security_param.get().into();
        let s = cryptarchia.config.s();
        maxvalid_bg(cryptarchia.local_chain, &cryptarchia.branches, k, s)
    }

    fn lib<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Id
    where
        Id: Eq + std::hash::Hash + Copy,
    {
        cryptarchia.branches.lib
    }
}

impl CryptarchiaState for Online {
    fn fork_choice<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Branch<Id>
    where
        Id: Eq + std::hash::Hash + Copy,
    {
        let k = cryptarchia.config.security_param.get().into();
        maxvalid_mc(cryptarchia.local_chain, &cryptarchia.branches, k)
    }

    fn lib<Id>(cryptarchia: &Cryptarchia<Id, Self>) -> Id
    where
        Id: Eq + std::hash::Hash + Copy,
    {
        cryptarchia
            .branches
            .nth_ancestor(
                &cryptarchia.local_chain,
                cryptarchia.config.security_param.get().into(),
            )
            .id()
    }
}

// Implementation of the fork choice rule as defined in the Ouroboros Genesis
// paper k defines the forking depth of chain we accept without more
// analysis s defines the length of time (unit of slots) after the fork
// happened we will inspect for chain density
fn maxvalid_bg<Id>(local_chain: Branch<Id>, branches: &Branches<Id>, k: u64, s: u64) -> Branch<Id>
where
    Id: Eq + std::hash::Hash + Copy,
{
    let mut cmax = local_chain;
    let forks = branches.branches();
    for chain in forks {
        let lowest_common_ancestor = branches.lca(&cmax, &chain);
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
            let cmax_density = branches.walk_back_before(&cmax, density_slot).length;
            let candidate_density = branches.walk_back_before(&chain, density_slot).length;
            if cmax_density < candidate_density {
                cmax = chain;
            }
        }
    }
    cmax
}

// Implementation of the fork choice rule as defined in the Ouroboros Praos
// paper k defines the forking depth of chain we can accept
fn maxvalid_mc<Id>(local_chain: Branch<Id>, branches: &Branches<Id>, k: u64) -> Branch<Id>
where
    Id: Eq + std::hash::Hash + Copy,
{
    let mut cmax = local_chain;
    let forks = branches.branches();
    for chain in forks {
        let lowest_common_ancestor = branches.lca(&cmax, &chain);
        let m = cmax.length - lowest_common_ancestor.length;
        if m <= k && cmax.length < chain.length {
            // Classic longest chain rule with parameter k
            cmax = chain;
        }
    }
    cmax
}

#[derive(Clone, Debug)]
pub struct Cryptarchia<Id, State: ?Sized> {
    local_chain: Branch<Id>,
    branches: Branches<Id>,
    config: Config,
    genesis: Id,
    // Just a marker to indicate whether the node is bootstrapping or online.
    // Does not actually end up in memory.
    _state: std::marker::PhantomData<State>,
}

impl<Id, State> PartialEq for Cryptarchia<Id, State>
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
    lib: Id,
}

impl<Id> PartialEq for Branches<Id>
where
    Id: Eq + Hash,
{
    fn eq(&self, other: &Self) -> bool {
        self.branches == other.branches && self.tips == other.tips && self.lib == other.lib
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

impl<Id> Branches<Id>
where
    Id: Eq + std::hash::Hash + Copy,
{
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
        let lib = genesis;
        Self {
            branches,
            tips,
            lib,
        }
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

        Self {
            branches,
            tips,
            lib: self.lib,
        }
    }

    /// Create a new [`Branches`] instance with the updated state.
    #[must_use = "this returns the result of the operation, without modifying the original"]
    fn apply_header(&self, header: Id, parent: Id, slot: Slot) -> Result<Self, Error<Id>> {
        let parent_branch = self
            .branches
            .get(&parent)
            .ok_or(Error::ParentMissing(parent))?;

        if parent_branch.slot > slot {
            return Err(Error::InvalidSlot(parent));
        }

        // TODO: we do not automatically prune forks here at the moment, so it's not
        // sufficient to check the header height.
        // We might relax this check in the future and get closer to the
        // Cryptarchia spec once we stabilize the pruning logic.
        if !self.is_ancestor(self.lib(), parent_branch) {
            return Err(Error::ImmutableFork(parent));
        }

        // Calculating the length here allows us to reuse
        // `Self::apply_header_unchecked`. Could this lead to length difference
        // issues? We are calculating length here but the `self.branches` is
        // cloned in the `Self::apply_header_unchecked` method, which means
        // there's a risk of length being different due to concurrent operations.
        let length = parent_branch.length + 1;

        Ok(self.apply_header_unchecked(header, parent, slot, length))
    }

    pub fn branches(&self) -> impl Iterator<Item = Branch<Id>> + use<'_, Id> {
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

    fn is_ancestor(&self, a: &Branch<Id>, b: &Branch<Id>) -> bool {
        let mut current = b;
        if a.id == b.id {
            return true; // `a` is the same as `b`
        }
        // Walk up the chain from `b` until we find `a` or reach the root
        while current.parent != current.id && current.length > a.length {
            if current.parent == a.id {
                return true; // Found `a` in the chain
            }
            current = &self.branches[&current.parent];
        }
        false // `a` is not an ancestor of `b`
    }

    // Returns the min(n, A)-th ancestor of the provided block, where A is the
    // number of ancestors of this block.
    fn nth_ancestor(&self, branch: &Branch<Id>, mut n: u64) -> Branch<Id> {
        let mut current = branch;
        while n > 0 {
            n -= 1;
            if let Some(parent) = self.branches.get(&current.parent) {
                current = parent;
            } else {
                return *current;
            }
        }
        *current
    }

    fn lib(&self) -> &Branch<Id> {
        &self.branches[&self.lib]
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
    Id: Eq + std::hash::Hash + Copy,
    State: CryptarchiaState + Copy,
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
            _state: std::marker::PhantomData,
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
        new.update_lib();
        Ok(new)
    }

    pub fn update_lib(&mut self) {
        self.branches.lib = <State as CryptarchiaState>::lib(&*self);
    }

    pub fn fork_choice(&self) -> Branch<Id> {
        <State as CryptarchiaState>::fork_choice(self)
    }

    pub const fn tip(&self) -> Id {
        self.local_chain.id
    }

    pub const fn tip_branch(&self) -> &Branch<Id> {
        &self.local_chain
    }

    /// Prune all blocks that are included in forks strictly older than 'depth'
    /// blocks from the current local chain.
    ///
    /// For example, if the tip of the canonical chain is at height 10, calling
    /// `self.prune_forks(10)` will remove any forks stemming from the genesis
    /// block, with height `0`.
    ///
    /// This function does not apply any particular logic when evaluating forks
    /// other than the height at which they diverged from the local
    /// canonical chain.
    ///
    /// It returns the block IDs that were part of the pruned forks.
    pub fn prune_forks(&mut self, depth: u64) -> impl Iterator<Item = Id> {
        let prunable_forks = self.prunable_forks(depth);
        let mut removed_blocks = vec![];
        for prunable_fork_info in prunable_forks {
            removed_blocks.extend(self.prune_fork(&prunable_fork_info));
        }
        removed_blocks.into_iter()
    }

    /// Get an iterator over the forks that can be pruned given the provided
    /// depth.
    ///
    /// This means that all forks that diverged from the canonical chain before
    /// the provided `depth` height are returned.
    pub fn prunable_forks(&self, depth: u64) -> impl Iterator<Item = ForkDivergenceInfo<Id>> {
        let local_chain = self.local_chain;
        let Some(target_height) = local_chain.length.checked_sub(depth) else {
            tracing::info!(
                target: LOG_TARGET,
                "No prunable fork, the canonical chain is not longer than the provided depth. Canonical chain length: {}, provided depth: {}", local_chain.length, depth
            );
            return vec![].into_iter();
        };
        self.non_canonical_forks()
            .filter_map(|fork| {
                // We calculate LCA once and store it in `ForkInfo` so it can be consumed
                // elsewhere without the need to re-calculate it.
                let lca = self.branches.lca(&local_chain, &fork);
                (lca.length <= target_height).then_some(ForkDivergenceInfo { tip: fork, lca })
            })
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// Returns all the forks that are not part of the local canonical chain.
    ///
    /// The result contains both prunable and non prunable forks.
    pub fn non_canonical_forks(&self) -> impl Iterator<Item = Branch<Id>> + '_ {
        self.branches
            .branches()
            .filter(|fork_tip| fork_tip.id != self.tip())
    }

    /// Remove all blocks from `tip` to `lca`, excluding `lca` which belongs to
    /// the canonical chain.
    fn prune_fork(
        &mut self,
        &ForkDivergenceInfo { lca, tip }: &ForkDivergenceInfo<Id>,
    ) -> impl Iterator<Item = Id> {
        let tip_removed = self.branches.tips.remove(&tip.id);
        if !tip_removed {
            tracing::error!(target: LOG_TARGET, "Fork tip {tip:#?} not found in the set of tips.");
        }

        let mut current_tip = tip.id;
        let mut removed_blocks = vec![];
        while current_tip != lca.id {
            let Some(branch) = self.branches.branches.remove(&current_tip) else {
                // If tip is not in branch set, it means this tip was sharing part of its
                // history with another fork that has already been removed.
                break;
            };
            removed_blocks.push(branch.id);
            current_tip = branch.parent;
        }
        tracing::debug!(
            target: LOG_TARGET,
            "Pruned {} branches from {tip:#?} to {current_tip:#?}.", removed_blocks.len()
        );
        removed_blocks.into_iter()
    }

    pub const fn genesis(&self) -> Id {
        self.genesis
    }

    pub const fn branches(&self) -> &Branches<Id> {
        &self.branches
    }

    /// Get the latest immutable block (LIB) in the chain. No re-orgs past this
    /// point are allowed.
    pub const fn lib(&self) -> Id {
        self.branches.lib
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

impl<Id> Cryptarchia<Id, Boostrapping>
where
    Id: Eq + std::hash::Hash + Copy,
{
    /// Signal transitioning to the online state.
    pub fn online(self) -> Cryptarchia<Id, Online> {
        let mut res = Cryptarchia {
            local_chain: self.local_chain,
            branches: self.branches.clone(),
            config: self.config,
            genesis: self.genesis,
            _state: std::marker::PhantomData,
        };
        // Update the LIB to the current local chain's tip
        res.update_lib();
        res
    }
}

#[cfg(test)]
pub mod tests {
    use std::{
        collections::HashSet,
        hash::{DefaultHasher, Hash, Hasher as _},
        num::NonZero,
    };

    use super::{maxvalid_bg, Boostrapping, Cryptarchia, Error, Slot};
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
    fn test_is_ancestor() {
        // parent
        // ├── child
        // │   ├── grandchild
        // │   └── granchild_2

        let mut branches = super::Branches::from_genesis([0; 32]);
        let parent = [1; 32];
        let child = [2; 32];
        let grandchild = [3; 32];
        let granchild_2: [u8; 32] = [4; 32];

        branches = branches.apply_header(parent, [0; 32], 1.into()).unwrap();
        branches = branches.apply_header(child, parent, 2.into()).unwrap();
        branches = branches.apply_header(grandchild, child, 3.into()).unwrap();
        branches = branches.apply_header(granchild_2, child, 4.into()).unwrap();

        assert!(branches.is_ancestor(
            branches.get(&parent).unwrap(),
            branches.get(&child).unwrap()
        ));
        assert!(!branches.is_ancestor(
            branches.get(&child).unwrap(),
            branches.get(&parent).unwrap()
        ));
        assert!(branches.is_ancestor(
            branches.get(&parent).unwrap(),
            branches.get(&grandchild).unwrap()
        ));
        assert!(!branches.is_ancestor(
            branches.get(&grandchild).unwrap(),
            branches.get(&parent).unwrap()
        ));
        assert!(branches.is_ancestor(
            branches.get(&child).unwrap(),
            branches.get(&grandchild).unwrap()
        ));
        assert!(!branches.is_ancestor(
            branches.get(&grandchild).unwrap(),
            branches.get(&child).unwrap()
        ));
        assert!(branches.is_ancestor(
            branches.get(&child).unwrap(),
            branches.get(&granchild_2).unwrap()
        ));
        assert!(!branches.is_ancestor(
            branches.get(&granchild_2).unwrap(),
            branches.get(&child).unwrap()
        ));
    }

    #[test]
    fn test_slot_increasing() {
        // parent
        // └── child

        let mut branches = super::Branches::from_genesis([0; 32]);
        let parent = [1; 32];
        let child = [2; 32];

        branches = branches.apply_header(parent, [0; 32], 2.into()).unwrap();
        assert!(matches!(
            branches.apply_header(child, parent, 1.into()),
            Err(Error::InvalidSlot(_))
        ));
    }

    #[test]
    fn test_immutable_fork() {
        // b1
        // └── LIB
        // |    └── b2
        // └── b3

        let mut branches = super::Branches::from_genesis([0; 32]);
        let b1 = [1; 32];
        let lib = [2; 32];
        let b2 = [3; 32];
        let b3 = [4; 32];
        branches = branches.apply_header(b1, [0; 32], 1.into()).unwrap();
        branches = branches.apply_header(lib, b1, 2.into()).unwrap();
        branches.lib = lib; // Set the LIB to b2
        branches = branches.apply_header(b2, lib, 3.into()).unwrap();
        assert!(matches!(
            branches.apply_header(b3, b1, 4.into()),
            Err(Error::ImmutableFork(_))
        ));
    }

    #[test]
    fn test_fork_choice() {
        // TODO: use cryptarchia
        let mut engine = <Cryptarchia<_, Boostrapping>>::from_genesis([0; 32], config());
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

        // however, if we set k to the fork length, it will be accepted
        let k = long_branch.length;
        assert_eq!(
            maxvalid_bg(*short_branch, engine.branches(), k, engine.config.s()).id,
            long_p
        );

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
        let engine = <Cryptarchia<_, Boostrapping>>::from_genesis([0; 32], config());
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
        let mut engine = <Cryptarchia<_, Boostrapping>>::from_genesis([0; 32], config());
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

    // It tests that nothing is pruned when the pruning depth is greater than the
    // canonical chain length.
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
        assert_eq!(pruned_blocks.collect::<HashSet<_>>(), [[100; 32]].into());
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
            pruned_blocks.collect::<HashSet<_>>(),
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
            pruned_blocks.collect::<HashSet<_>>(),
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
