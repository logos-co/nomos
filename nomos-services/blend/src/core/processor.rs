use std::{
    hash::Hash,
    num::NonZeroU64,
    ops::{Deref, DerefMut},
};

use nomos_blend_scheduling::{
    membership::Membership,
    message_blend::{
        crypto::send_and_receive::SessionCryptographicProcessor,
        ProofsGenerator as ProofsGeneratorTrait, SessionCryptographicProcessorSettings,
        SessionInfo,
    },
};

pub struct CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>(
    SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>,
);

impl<NodeId, ProofsGenerator, ProofsVerifier>
    CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    ProofsGenerator: ProofsGeneratorTrait,
{
    pub fn try_new_with_core_condition_check(
        membership: Membership<NodeId>,
        minimum_network_size: NonZeroU64,
        settings: &SessionCryptographicProcessorSettings,
        session_info: SessionInfo,
        proofs_verifier: ProofsVerifier,
    ) -> Result<Self, Error>
    where
        NodeId: Eq + Hash,
    {
        if membership.size() < minimum_network_size.get() as usize {
            Err(Error::NetworkIsTooSmall(membership.size()))
        } else if !membership.contains_local() {
            Err(Error::LocalIsNotCoreNode)
        } else {
            Ok(Self::new(
                membership,
                session_info,
                settings,
                proofs_verifier,
            ))
        }
    }

    fn new(
        membership: Membership<NodeId>,
        session_info: SessionInfo,
        settings: &SessionCryptographicProcessorSettings,
        proofs_verifier: ProofsVerifier,
    ) -> Self {
        Self(SessionCryptographicProcessor::new(
            settings,
            membership,
            session_info,
            proofs_verifier,
        ))
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier>
    CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
{
    pub fn into_inner(
        self,
    ) -> SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier> {
        self.0
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier> Deref
    for CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
{
    type Target = SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier> DerefMut
    for CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
{
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
    use crate::test_utils::{
        crypto::{MockProofsGenerator, MockProofsVerifier},
        membership::{key, membership, mock_session_info},
    };

    #[test]
    fn try_new_with_valid_membership() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(1)];
        CoreCryptographicProcessor::<_, MockProofsGenerator, _>::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(1).unwrap(),
            &settings(local_id),
            mock_session_info(),
            MockProofsVerifier,
        )
        .unwrap();
    }

    #[test]
    fn try_new_with_small_membership() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(1)];
        let result = CoreCryptographicProcessor::<_, MockProofsGenerator, _>::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(2).unwrap(),
            &settings(local_id),
            mock_session_info(),
            MockProofsVerifier
        );
        assert!(matches!(result, Err(Error::NetworkIsTooSmall(1))));
    }

    #[test]
    fn try_new_with_local_node_not_core() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(2)];
        let result = CoreCryptographicProcessor::<_, MockProofsGenerator, _>::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(1).unwrap(),
            &settings(local_id),
            mock_session_info(),
            MockProofsVerifier
        );
        assert!(matches!(result, Err(Error::LocalIsNotCoreNode)));
    }

    fn settings(local_id: NodeId) -> SessionCryptographicProcessorSettings {
        SessionCryptographicProcessorSettings {
            non_ephemeral_signing_key: key(local_id).0,
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
