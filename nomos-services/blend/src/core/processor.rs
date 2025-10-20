use std::{
    hash::Hash,
    num::NonZeroU64,
    ops::{Deref, DerefMut},
};

use nomos_blend_message::{
    crypto::proofs::{
        PoQVerificationInputsMinusSigningKey, quota::inputs::prove::private::ProofOfCoreQuotaInputs,
    },
    encap::ProofsVerifier as ProofsVerifierTrait,
};
use nomos_blend_scheduling::{
    membership::Membership,
    message_blend::{
        crypto::{
            SessionCryptographicProcessorSettings,
            core_and_leader::send_and_receive::SessionCryptographicProcessor,
        },
        provers::core_and_leader::CoreAndLeaderProofsGenerator,
    },
};

pub struct CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>(
    SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>,
);

impl<NodeId, ProofsGenerator, ProofsVerifier>
    CoreCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    ProofsGenerator: CoreAndLeaderProofsGenerator,
    ProofsVerifier: ProofsVerifierTrait,
{
    pub fn try_new_with_core_condition_check(
        membership: Membership<NodeId>,
        minimum_network_size: NonZeroU64,
        settings: &SessionCryptographicProcessorSettings,
        public_info: PoQVerificationInputsMinusSigningKey,
        private_core_info: ProofOfCoreQuotaInputs,
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
                settings,
                public_info,
                private_core_info,
            ))
        }
    }

    fn new(
        membership: Membership<NodeId>,
        settings: &SessionCryptographicProcessorSettings,
        public_info: PoQVerificationInputsMinusSigningKey,
        private_core_info: ProofOfCoreQuotaInputs,
    ) -> Self {
        Self(SessionCryptographicProcessor::new(
            settings,
            membership,
            public_info,
            private_core_info,
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
    use nomos_blend_message::crypto::proofs::quota::inputs::prove::{
        private::ProofOfLeadershipQuotaInputs,
        public::{CoreInputs, LeaderInputs},
    };
    use nomos_blend_scheduling::message_blend::provers::{
        BlendLayerProof, ProofsGeneratorSettings,
    };
    use nomos_core::crypto::ZkHash;

    use super::*;
    use crate::test_utils::{
        crypto::{MockProofsVerifier, mock_blend_proof},
        membership::{key, membership},
    };

    pub struct MockCoreAndLeaderProofsGenerator;

    #[async_trait]
    impl CoreAndLeaderProofsGenerator for MockCoreAndLeaderProofsGenerator {
        fn new(
            _settings: ProofsGeneratorSettings,
            _private_inputs: ProofOfCoreQuotaInputs,
        ) -> Self {
            Self
        }

        fn rotate_epoch(&mut self, _new_epoch_public: LeaderInputs) {}

        fn set_epoch_private(&mut self, _new_epoch_private: ProofOfLeadershipQuotaInputs) {}

        async fn get_next_core_proof(&mut self) -> Option<BlendLayerProof> {
            Some(mock_blend_proof())
        }

        async fn get_next_leader_proof(&mut self) -> Option<BlendLayerProof> {
            Some(mock_blend_proof())
        }
    }

    fn mock_verification_inputs() -> PoQVerificationInputsMinusSigningKey {
        use groth16::Field as _;

        PoQVerificationInputsMinusSigningKey {
            session: 1,
            core: CoreInputs {
                quota: 1,
                zk_root: ZkHash::ZERO,
            },
            leader: LeaderInputs {
                pol_ledger_aged: ZkHash::ZERO,
                pol_epoch_nonce: ZkHash::ZERO,
                message_quota: 1,
                total_stake: 1,
            },
        }
    }

    fn mock_core_poq_inputs() -> ProofOfCoreQuotaInputs {
        use groth16::Field as _;

        ProofOfCoreQuotaInputs {
            core_sk: ZkHash::ZERO,
            core_path_and_selectors: [(ZkHash::ZERO, false); _],
        }
    }

    #[test]
    fn try_new_with_valid_membership() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(1)];
        CoreCryptographicProcessor::<_, MockCoreAndLeaderProofsGenerator, MockProofsVerifier>::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(1).unwrap(),
            &settings(local_id),
            mock_verification_inputs(),
            mock_core_poq_inputs()
        )
        .unwrap();
    }

    #[test]
    fn try_new_with_small_membership() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(1)];
        let result = CoreCryptographicProcessor::<
            _,
            MockCoreAndLeaderProofsGenerator,
            MockProofsVerifier,
        >::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(2).unwrap(),
            &settings(local_id),
            mock_verification_inputs(),
            mock_core_poq_inputs(),
        );
        assert!(matches!(result, Err(Error::NetworkIsTooSmall(1))));
    }

    #[test]
    fn try_new_with_local_node_not_core() {
        let local_id = NodeId(1);
        let core_nodes = [NodeId(2)];
        let result = CoreCryptographicProcessor::<
            _,
            MockCoreAndLeaderProofsGenerator,
            MockProofsVerifier,
        >::try_new_with_core_condition_check(
            membership(&core_nodes, local_id),
            NonZeroU64::new(1).unwrap(),
            &settings(local_id),
            mock_verification_inputs(),
            mock_core_poq_inputs(),
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
