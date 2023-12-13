use crate::{Committee, CommitteeId, NodeId};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(super) struct Tree {
    pub(super) inner_committees: Vec<CommitteeId>,
    pub(super) membership_committees: HashMap<usize, Committee>,
    pub(super) committee_id_to_index: HashMap<CommitteeId, usize>,
    pub(super) committees_by_member: HashMap<NodeId, usize>,
}

impl Tree {
    pub fn new(nodes: &[NodeId], number_of_committees: usize) -> Self {
        assert!(number_of_committees > 0);

        let (inner_committees, membership_committees) =
            Self::build_committee_from_nodes_with_size(nodes, number_of_committees);

        let committee_id_to_index = inner_committees
            .iter()
            .copied()
            .enumerate()
            .map(|(idx, c)| (c, idx))
            .collect();

        let committees_by_member = membership_committees
            .iter()
            .flat_map(|(committee, members)| members.iter().map(|member| (*member, *committee)))
            .collect();

        Self {
            inner_committees,
            membership_committees,
            committee_id_to_index,
            committees_by_member,
        }
    }

    pub(super) fn build_committee_from_nodes_with_size(
        nodes: &[NodeId],
        number_of_committees: usize,
    ) -> (Vec<CommitteeId>, HashMap<usize, Committee>) {
        let committee_size = nodes.len() / number_of_committees;
        let remainder = nodes.len() % number_of_committees;

        let mut committees: Vec<Committee> = (0..number_of_committees)
            .map(|n| {
                nodes[n * committee_size..(n + 1) * committee_size]
                    .iter()
                    .cloned()
                    .collect()
            })
            .collect();

        // Refill committees with extra nodes
        if remainder != 0 {
            for i in 0..remainder {
                let node = nodes[nodes.len() - remainder + i];
                let committee_index = i % number_of_committees;
                committees[committee_index].insert(node);
            }
        }

        let hashes = committees
            .iter()
            .map(Committee::id::<blake2::Blake2b<digest::typenum::U32>>)
            .collect::<Vec<_>>();
        (hashes, committees.into_iter().enumerate().collect())
    }

    pub(super) fn parent_committee(&self, committee_id: &CommitteeId) -> Option<&CommitteeId> {
        if committee_id == &self.inner_committees[0] {
            None
        } else {
            self.committee_id_to_index
                .get(committee_id)
                .map(|&idx| &self.inner_committees[(idx - 1) / 2])
        }
    }

    pub(super) fn parent_committee_from_member_id(&self, id: &NodeId) -> Option<Committee> {
        let committee_id = self.committee_id_by_member_id(id)?;
        let parent_id = self.parent_committee(committee_id)?;
        self.committee_by_committee_idx(self.committee_id_to_index[parent_id])
            .cloned()
    }

    pub(super) fn child_committees(
        &self,
        committee_id: &CommitteeId,
    ) -> (Option<&CommitteeId>, Option<&CommitteeId>) {
        let Some(base) = self
            .committee_id_to_index
            .get(committee_id)
            .map(|&idx| idx * 2)
        else {
            return (None, None);
        };
        let first_child = base + 1;
        let second_child = base + 2;
        (
            self.inner_committees.get(first_child),
            self.inner_committees.get(second_child),
        )
    }

    pub(super) fn leaf_committees(&self) -> HashMap<&CommitteeId, &Committee> {
        let total_leafs = (self.inner_committees.len() + 1) / 2;
        let mut leaf_committees = HashMap::new();
        for i in (self.inner_committees.len() - total_leafs)..self.inner_committees.len() {
            leaf_committees.insert(&self.inner_committees[i], &self.membership_committees[&i]);
        }
        leaf_committees
    }

    pub(super) fn root_committee(&self) -> &Committee {
        &self.membership_committees[&0]
    }

    pub(super) fn committee_by_committee_idx(&self, committee_idx: usize) -> Option<&Committee> {
        self.membership_committees.get(&committee_idx)
    }

    pub(super) fn committee_idx_by_member_id(&self, member_id: &NodeId) -> Option<usize> {
        self.committees_by_member.get(member_id).copied()
    }

    pub(super) fn committee_id_by_member_id(&self, member_id: &NodeId) -> Option<&CommitteeId> {
        self.committees_by_member
            .get(member_id)
            .map(|&idx| &self.inner_committees[idx])
    }

    pub(super) fn committee_by_member_id(&self, member_id: &NodeId) -> Option<&Committee> {
        self.committee_idx_by_member_id(member_id)
            .and_then(|idx| self.committee_by_committee_idx(idx))
    }

    #[allow(dead_code)]
    pub(super) fn committee_by_committee_id(
        &self,
        committee_id: &CommitteeId,
    ) -> Option<&Committee> {
        self.committee_id_to_index
            .get(committee_id)
            .and_then(|&idx| self.committee_by_committee_idx(idx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_carnot_tree_parenting() {
        let nodes: Vec<_> = (0..10).map(|i| NodeId::new([i as u8; 32])).collect();
        let tree = Tree::new(&nodes, 3);

        let root = &tree.inner_committees[0];
        let one = &tree.inner_committees[1];
        let two = &tree.inner_committees[2];

        assert_eq!(tree.parent_committee(one), Some(root));
        assert_eq!(tree.parent_committee(two), Some(root));
    }

    #[test]
    fn test_carnot_tree_root_parenting() {
        let nodes: Vec<_> = (0..10).map(|i| NodeId::new([i as u8; 32])).collect();
        let tree = Tree::new(&nodes, 3);

        let root = &tree.inner_committees[0];

        assert!(tree.parent_committee(root).is_none());
    }

    #[test]
    fn test_carnot_tree_childs() {
        let nodes: Vec<_> = (0..10).map(|i| NodeId::new([i as u8; 32])).collect();
        let tree = Tree::new(&nodes, 3);

        let root = &tree.inner_committees[0];
        let one = &tree.inner_committees[1];
        let two = &tree.inner_committees[2];

        assert_eq!(tree.child_committees(root), (Some(one), Some(two)));
    }

    #[test]
    fn test_carnot_tree_leaf_parents() {
        let nodes: Vec<_> = (0..10).map(|i| NodeId::new([i as u8; 32])).collect();
        let tree = Tree::new(&nodes, 7);

        // Helper function to get nodes from a committee.
        fn get_nodes_from_committee(tree: &Tree, committee: &CommitteeId) -> Vec<NodeId> {
            tree.committee_by_committee_id(committee)
                .unwrap()
                .iter()
                .cloned()
                .collect()
        }

        // Helper function to test committee and parent relationship.
        fn test_committee_parent(tree: &Tree, child: &CommitteeId, parent: &CommitteeId) {
            let child_nodes = get_nodes_from_committee(tree, child);
            let node_comm = tree.committee_by_member_id(&child_nodes[0]).unwrap();
            assert_eq!(
                node_comm.id::<blake2::Blake2b<digest::typenum::U32>>(),
                *child
            );
            assert_eq!(
                tree.parent_committee_from_member_id(&child_nodes[0])
                    .map(|c| c.id::<blake2::Blake2b<digest::typenum::U32>>()),
                Some(*parent)
            );
        }

        // (Upper Committee, (Leaf 1, Leaf 2))
        let test_cases = [
            (
                &tree.inner_committees[1],
                (
                    Some(&tree.inner_committees[3]),
                    Some(&tree.inner_committees[4]),
                ),
            ),
            (
                &tree.inner_committees[2],
                (
                    Some(&tree.inner_committees[5]),
                    Some(&tree.inner_committees[6]),
                ),
            ),
        ];
        test_cases.iter().for_each(|tcase| {
            let child_comm = tree.child_committees(tcase.0);
            assert_eq!(child_comm, tcase.1);
            test_committee_parent(&tree, tcase.1 .0.unwrap(), tcase.0);
            test_committee_parent(&tree, tcase.1 .1.unwrap(), tcase.0);
        });
    }
}
