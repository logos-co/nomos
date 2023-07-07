use std::collections::BTreeSet;

use crate::NodeId;

#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CommitteeId(pub(crate) [u8; 32]);

impl CommitteeId {
    pub const fn new(val: [u8; 32]) -> Self {
        Self(val)
    }
}

impl From<[u8; 32]> for CommitteeId {
    fn from(id: [u8; 32]) -> Self {
        Self(id)
    }
}

impl From<&[u8; 32]> for CommitteeId {
    fn from(id: &[u8; 32]) -> Self {
        Self(*id)
    }
}

impl From<CommitteeId> for [u8; 32] {
    fn from(id: CommitteeId) -> Self {
        id.0
    }
}

impl core::fmt::Display for CommitteeId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x")?;
        for v in self.0 {
            write!(f, "{:02x}", v)?;
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct Committee {
    members: BTreeSet<NodeId>,
}

impl Committee {
    #[inline]
    pub const fn new() -> Self {
        Self {
            members: BTreeSet::new(),
        }
    }

    #[inline]
    pub fn hash<D: digest::Digest>(
        &self,
    ) -> digest::generic_array::GenericArray<u8, <D as digest::OutputSizeUser>::OutputSize> {
        let mut hasher = D::new();
        for member in &self.members {
            hasher.update(member);
        }
        hasher.finalize()
    }

    #[inline]
    pub fn contains(&self, node_id: &NodeId) -> bool {
        self.members.contains(node_id)
    }

    #[inline]
    pub fn insert(&mut self, node_id: NodeId) {
        self.members.insert(node_id);
    }

    #[inline]
    pub fn remove(&mut self, node_id: &NodeId) {
        self.members.remove(node_id);
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.members.len()
    }

    #[inline]
    pub fn extend<'a>(&mut self, other: impl IntoIterator<Item = &'a NodeId>) {
        self.members.extend(other);
    }

    #[inline]
    pub fn id<D: digest::Digest<OutputSize = digest::typenum::U32>>(&self) -> CommitteeId {
        CommitteeId::new(self.hash::<D>().into())
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &NodeId> {
        self.members.iter()
    }
}

impl<'a, T> From<T> for Committee
where
    T: Iterator<Item = &'a NodeId>,
{
    fn from(members: T) -> Self {
        Self {
            members: members.into_iter().copied().collect(),
        }
    }
}

impl core::iter::FromIterator<[u8; 32]> for Committee {
    fn from_iter<T: IntoIterator<Item = [u8; 32]>>(iter: T) -> Self {
        Self {
            members: iter.into_iter().collect(),
        }
    }
}

impl<'a> core::iter::FromIterator<&'a [u8; 32]> for Committee {
    fn from_iter<T: IntoIterator<Item = &'a [u8; 32]>>(iter: T) -> Self {
        Self {
            members: iter.into_iter().copied().collect(),
        }
    }
}

impl core::iter::IntoIterator for Committee {
    type Item = NodeId;

    type IntoIter = std::collections::btree_set::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.members.into_iter()
    }
}

impl<'a> core::iter::IntoIterator for &'a Committee {
    type Item = &'a NodeId;

    type IntoIter = std::collections::btree_set::Iter<'a, NodeId>;

    fn into_iter(self) -> Self::IntoIter {
        self.members.iter()
    }
}
