use core::ops::{Deref, DerefMut};

use nomos_blend_message::{
    Error,
    crypto::proofs::quota::inputs::prove::private::ProofOfCoreQuotaInputs,
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
        provers::{ProofsGeneratorSettings, core_and_leader::CoreAndLeaderProofsGenerator},
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
{
    #[must_use]
    pub fn new(
        settings: &SessionCryptographicProcessorSettings,
        membership: Membership<NodeId>,
        public_core_info: ProofsGeneratorSettings,
        private_core_info: ProofOfCoreQuotaInputs,
        proofs_verifier: ProofsVerifier,
    ) -> Self {
        Self {
            sender_processor: SenderSessionCryptographicProcessor::new(
                settings,
                membership,
                public_core_info,
                private_core_info,
            ),
            proofs_verifier,
        }
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
