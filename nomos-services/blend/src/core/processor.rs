use std::{
    hash::Hash,
    num::NonZeroU64,
    ops::{Deref, DerefMut},
};

use nomos_blend_message::encap::ProofsVerifier as ProofsVerifierTrait;
use nomos_blend_scheduling::{
    membership::Membership,
    message_blend::{
        crypto::SessionCryptographicProcessor, CryptographicProcessorSettings,
        ProofsGenerator as ProofsGeneratorTrait, SessionInfo,
    },
};

pub struct CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>(
    SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>,
);

impl<NodeId, ProofsGenerator, ProofsVerifier>
    CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    ProofsGenerator: ProofsGeneratorTrait,
    ProofsVerifier: ProofsVerifierTrait,
{
    pub fn try_new_with_core_condition_check(
        membership: Membership<NodeId>,
        minimum_network_size: NonZeroU64,
        settings: &CryptographicProcessorSettings,
        session_info: SessionInfo,
    ) -> Result<Self, Error>
    where
        NodeId: Eq + Hash,
    {
        if membership.size() < minimum_network_size.get() as usize {
            Err(Error::NetworkIsTooSmall(membership.size()))
        } else if !membership.contains_local() {
            Err(Error::LocalIsNotCoreNode)
        } else {
            Ok(Self::new(membership, session_info, settings.clone()))
        }
    }

    fn new(
        membership: Membership<NodeId>,
        session_info: SessionInfo,
        settings: CryptographicProcessorSettings,
    ) -> Self {
        Self(SessionCryptographicProcessor::new(
            settings,
            membership,
            session_info,
        ))
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
    use async_trait::async_trait;
    use groth16::Field as _;
    use nomos_blend_scheduling::message_blend::{BlendProof, PrivateInfo, PublicInfo};
    use nomos_core::crypto::ZkHash;

    use super::*;
    use crate::test_utils::{
        crypto::NeverFailingProofsVerifier,
        membership::{key, membership},
    };

    fn mock_session_info() -> SessionInfo {
        SessionInfo {
            public: PublicInfo {
                core_quota: 0,
                core_root: ZkHash::ZERO,
                leader_quota: 0,
                pol_epoch_nonce: ZkHash::ZERO,
                pol_ledger_aged: ZkHash::ZERO,
                session: 0,
                total_stake: 0,
            },
            private: PrivateInfo {
                aged_path: vec![],
                aged_selector: vec![],
                core_path: vec![],
                core_path_selectors: vec![],
                core_sk: ZkHash::ZERO,
                note_value: 0,
                output_number: 0,
                pol_secret_key: ZkHash::ZERO,
                slot: 0,
                slot_secret: ZkHash::ZERO,
                slot_secret_path: vec![],
                starting_slot: 0,
                transaction_hash: ZkHash::ZERO,
            },
        }
    }

    struct MockProofsGenerator;

    #[async_trait]
    impl ProofsGeneratorTrait for MockProofsGenerator {
        fn new(_session_info: SessionInfo) -> Self {
            Self
        }

        async fn get_next_core_proof(&mut self) -> Option<BlendProof> {
            None
        }

        async fn get_next_leadership_proof(&mut self) -> Option<BlendProof> {
            None
        }
    }

    #[test]
    fn try_new_with_valid_membership() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(1)];
        CoreCryptographicProcessor::<_, MockProofsGenerator, NeverFailingProofsVerifier>::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(1).unwrap(),
            &settings(local_id),
            mock_session_info(),
        )
        .unwrap();
    }

    #[test]
    fn try_new_with_small_membership() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(1)];
        let result = CoreCryptographicProcessor::<_, MockProofsGenerator, NeverFailingProofsVerifier>::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(2).unwrap(),
            &settings(local_id),
            mock_session_info(),
        );
        assert!(matches!(result, Err(Error::NetworkIsTooSmall(1))));
    }

    #[test]
    fn try_new_with_local_node_not_core() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(2)];
        let result = CoreCryptographicProcessor::<_, MockProofsGenerator, NeverFailingProofsVerifier>::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(1).unwrap(),
            &settings(local_id),
            mock_session_info(),
        );
        assert!(matches!(result, Err(Error::LocalIsNotCoreNode)));
    }

    fn settings(local_id: NodeId) -> CryptographicProcessorSettings {
        CryptographicProcessorSettings {
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
