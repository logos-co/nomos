#![allow(
    clippy::disallowed_script_idents,
    reason = "The crate `cfg_eval` contains Sinhala script identifiers. \
    Using the `expect` or `allow` macro on top of their usage does not remove the warning"
)]

pub mod config;
pub mod time;

use std::collections::{HashMap, HashSet};

pub use config::*;
use thiserror::Error;
pub use time::{Epoch, EpochConfig, Slot};

#[derive(Clone, Debug)]
pub struct Cryptarchia<Id> {
    local_chain: Branch<Id>,
    branches: Branches<Id>,
    config: Config,
    genesis: Id,
}

#[derive(Clone, Debug)]
pub struct Branches<Id> {
    branches: HashMap<Id, Branch<Id>>,
    tips: HashSet<Id>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
            + 1;

        Ok(self.apply_header_unchecked(header, parent, slot, length))
    }

    #[must_use]
    pub fn branches(&self) -> Vec<Branch<Id>> {
        self.tips
            .iter()
            .map(|id| self.branches[id].clone())
            .collect()
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

        b1.clone()
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
        current.clone()
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
    Id: Eq + std::hash::Hash + Copy,
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
        Self::maxvalid_bg(self.local_chain.clone(), &self.branches, k, s)
    }

    pub const fn tip(&self) -> Id {
        self.local_chain.id
    }

    // prune all states deeper than 'depth' with regard to the current
    // local chain except for states belonging to the local chain
    pub fn prune_forks(&mut self, _depth: u64) {
        todo!()
    }

    pub const fn genesis(&self) -> Id {
        self.genesis
    }

    pub const fn branches(&self) -> &Branches<Id> {
        &self.branches
    }

    //  Implementation of the fork choice rule as defined in the Ouroboros Genesis
    // paper  k defines the forking depth of chain we accept without more
    // analysis  s defines the length of time (unit of slots) after the fork
    // happened we will inspect for chain density
    fn maxvalid_bg(local_chain: Branch<Id>, branches: &Branches<Id>, k: u64, s: u64) -> Branch<Id> {
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

        let bs = engine.branches().branches();
        let long_branch = bs.iter().find(|b| b.id == long_p).unwrap();
        let short_branch = bs.iter().find(|b| b.id == short_p).unwrap();
        assert!(long_branch.length > short_branch.length);

        // however, if we set k to the fork length, it will be accepted
        let k = long_branch.length;
        assert_eq!(
            Cryptarchia::maxvalid_bg(
                short_branch.clone(),
                engine.branches(),
                k,
                engine.config.s()
            )
            .id,
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

    fn hash<T: Hash>(t: &T) -> [u8; 32] {
        let mut s = DefaultHasher::new();
        t.hash(&mut s);
        let hash = s.finish();
        let mut res = [0; 32];
        res[..8].copy_from_slice(&hash.to_be_bytes());
        res
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
}
