use std::{
    hash::Hash,
    num::NonZeroU64,
    ops::{Deref, DerefMut},
};

use nomos_blend_scheduling::{
    membership::Membership,
    message_blend::{crypto::CryptographicProcessor, CryptographicProcessorSettings},
};
use nomos_utils::blake_rng::BlakeRng;
use rand::SeedableRng as _;

pub struct CoreCryptographicProcessor<NodeId>(CryptographicProcessor<NodeId, BlakeRng>);

impl<NodeId> CoreCryptographicProcessor<NodeId> {
    pub fn try_new_with_core_condition_check(
        membership: Membership<NodeId>,
        minimum_network_size: NonZeroU64,
        settings: &CryptographicProcessorSettings,
    ) -> Result<Self, Error>
    where
        NodeId: Eq + Hash,
    {
        if membership.size() < minimum_network_size.get() as usize {
            Err(Error::NetworkIsTooSmall(membership.size()))
        } else if !membership.contains_local() {
            Err(Error::LocalIsNotCoreNode)
        } else {
            Ok(Self::new(membership, settings.clone()))
        }
    }

    fn new(membership: Membership<NodeId>, settings: CryptographicProcessorSettings) -> Self {
        Self(CryptographicProcessor::new(
            settings,
            membership,
            BlakeRng::from_entropy(),
        ))
    }
}

impl<NodeId> Deref for CoreCryptographicProcessor<NodeId> {
    type Target = CryptographicProcessor<NodeId, BlakeRng>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<NodeId> DerefMut for CoreCryptographicProcessor<NodeId> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Network is too small: {0}")]
    NetworkIsTooSmall(usize),
    #[error("Local node is not a core node")]
    LocalIsNotCoreNode,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::membership::{key, membership};

    #[test]
    fn try_new_with_valid_membership() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(1)];
        CoreCryptographicProcessor::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(1).unwrap(),
            &settings(local_id),
        )
        .unwrap();
    }

    #[test]
    fn try_new_with_small_membership() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(1)];
        let result = CoreCryptographicProcessor::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(2).unwrap(),
            &settings(local_id),
        );
        assert!(matches!(result, Err(Error::NetworkIsTooSmall(1))));
    }

    #[test]
    fn try_new_with_local_node_not_core() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(2)];
        let result = CoreCryptographicProcessor::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(1).unwrap(),
            &settings(local_id),
        );
        assert!(matches!(result, Err(Error::LocalIsNotCoreNode)));
    }

    fn settings(local_id: NodeId) -> CryptographicProcessorSettings {
        CryptographicProcessorSettings {
            signing_private_key: key(local_id).0,
            num_blend_layers: 1,
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
    struct NodeId(u8);

    impl From<NodeId> for [u8; 32] {
        fn from(id: NodeId) -> Self {
            [id.0; 32]
        }
    }
}
