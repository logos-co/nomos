use core::ops::{Deref, DerefMut};

use nomos_blend_message::{
    Error,
    crypto::proofs::{
        PoQVerificationInputsMinusSigningKey,
        quota::inputs::prove::{private::ProofOfCoreQuotaInputs, public::LeaderInputs},
    },
    encap::{
        ProofsVerifier as ProofsVerifierTrait,
        validated::RequiredProofOfSelectionVerificationInputs,
    },
};

use crate::{
    DecapsulationOutput,
    membership::Membership,
    message_blend::{
        crypto::{
            IncomingEncapsulatedMessageWithValidatedPublicHeader,
            SessionCryptographicProcessorSettings,
            core_and_leader::send::SessionCryptographicProcessor as SenderSessionCryptographicProcessor,
        },
        provers::core_and_leader::CoreAndLeaderProofsGenerator,
    },
};

/// [`SessionCryptographicProcessor`] is responsible for wrapping both cover and
/// data messages and unwrapping messages for the message indistinguishability.
///
/// Each instance is meant to be used during a single session.
///
/// This processor is suitable for core nodes.
pub struct SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier> {
    sender_processor: SenderSessionCryptographicProcessor<NodeId, ProofsGenerator>,
    proofs_verifier: ProofsVerifier,
}

impl<NodeId, ProofsGenerator, ProofsVerifier>
    SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    ProofsGenerator: CoreAndLeaderProofsGenerator,
    ProofsVerifier: ProofsVerifierTrait,
{
    #[must_use]
    pub fn new(
        settings: &SessionCryptographicProcessorSettings,
        membership: Membership<NodeId>,
        public_info: PoQVerificationInputsMinusSigningKey,
        private_core_info: ProofOfCoreQuotaInputs,
    ) -> Self {
        Self {
            sender_processor: SenderSessionCryptographicProcessor::new(
                settings,
                membership,
                public_info,
                private_core_info,
            ),
            proofs_verifier: ProofsVerifier::new(public_info),
        }
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier>
    SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    ProofsGenerator: CoreAndLeaderProofsGenerator,
    ProofsVerifier: ProofsVerifierTrait,
{
    pub fn rotate_epoch(&mut self, new_epoch_public: LeaderInputs) {
        self.sender_processor.rotate_epoch(new_epoch_public);
        self.proofs_verifier
            .start_epoch_transition(new_epoch_public);
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier>
    SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    ProofsVerifier: ProofsVerifierTrait,
{
    pub fn complete_epoch_transition(&mut self) {
        self.proofs_verifier.complete_epoch_transition();
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier>
    SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
{
    pub fn take_verifier(self) -> ProofsVerifier {
        self.proofs_verifier
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier>
    SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    ProofsVerifier: ProofsVerifierTrait,
{
    pub fn decapsulate_message(
        &self,
        message: IncomingEncapsulatedMessageWithValidatedPublicHeader,
    ) -> Result<DecapsulationOutput, Error> {
        let Some(local_node_index) = self.sender_processor.membership().local_index() else {
            return Err(Error::NotCoreNodeReceiver);
        };
        message.decapsulate(
            self.sender_processor.non_ephemeral_encryption_key(),
            &RequiredProofOfSelectionVerificationInputs {
                expected_node_index: local_node_index as u64,
                total_membership_size: self.sender_processor.membership().size() as u64,
            },
            &self.proofs_verifier,
        )
    }
}

// `Deref` and `DerefMut` so we can call the `encapsulate*` methods exposed by
// the send-only processor.
impl<NodeId, ProofsGenerator, ProofsVerifier> Deref
    for SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
{
    type Target = SenderSessionCryptographicProcessor<NodeId, ProofsGenerator>;

    fn deref(&self) -> &Self::Target {
        &self.sender_processor
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier> DerefMut
    for SessionCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.sender_processor
    }
}

#[cfg(test)]
mod test {
    use groth16::Field as _;
    use multiaddr::{Multiaddr, PeerId};
    use nomos_blend_message::crypto::{
        keys::Ed25519PrivateKey,
        proofs::{
            PoQVerificationInputsMinusSigningKey,
            quota::inputs::prove::{
                private::ProofOfCoreQuotaInputs,
                public::{CoreInputs, LeaderInputs},
            },
        },
    };
    use nomos_core::crypto::ZkHash;

    use super::SessionCryptographicProcessor;
    use crate::{
        membership::{Membership, Node},
        message_blend::crypto::{
            SessionCryptographicProcessorSettings,
            test_utils::{
                TestEpochChangeCoreAndLeaderProofsGenerator, TestEpochChangeProofsVerifier,
            },
        },
    };

    #[test]
    fn epoch_rotation() {
        let mut processor = SessionCryptographicProcessor::<
            _,
            TestEpochChangeCoreAndLeaderProofsGenerator,
            TestEpochChangeProofsVerifier,
        >::new(
            &SessionCryptographicProcessorSettings {
                non_ephemeral_signing_key: Ed25519PrivateKey::generate(),
                num_blend_layers: 1,
            },
            Membership::new_without_local(&[Node {
                address: Multiaddr::empty(),
                id: PeerId::random(),
                public_key: [0; _].try_into().unwrap(),
            }]),
            PoQVerificationInputsMinusSigningKey {
                session: 1,
                core: CoreInputs {
                    quota: 1,
                    zk_root: ZkHash::ZERO,
                },
                leader: LeaderInputs {
                    message_quota: 1,
                    pol_epoch_nonce: ZkHash::ZERO,
                    pol_ledger_aged: ZkHash::ZERO,
                    total_stake: 1,
                },
            },
            ProofOfCoreQuotaInputs {
                core_path_and_selectors: [(ZkHash::ZERO, false); _],
                core_sk: ZkHash::ZERO,
            },
        );

        let new_leader_inputs = LeaderInputs {
            pol_ledger_aged: ZkHash::ONE,
            pol_epoch_nonce: ZkHash::ONE,
            message_quota: 2,
            total_stake: 2,
        };

        processor.rotate_epoch(new_leader_inputs);

        assert_eq!(processor.proofs_verifier.0.leader, new_leader_inputs);
        assert_eq!(
            processor.proofs_verifier.1,
            Some(LeaderInputs {
                message_quota: 1,
                pol_epoch_nonce: ZkHash::ZERO,
                pol_ledger_aged: ZkHash::ZERO,
                total_stake: 1,
            })
        );
        assert_eq!(
            processor.proofs_generator().0.public_inputs.leader,
            new_leader_inputs
        );

        processor.complete_epoch_transition();

        assert!(processor.proofs_verifier.1.is_none());
    }
}
