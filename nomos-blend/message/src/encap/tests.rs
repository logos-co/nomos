use core::convert::Infallible;

use nomos_core::crypto::ZkHash;

use crate::{
    Error, PayloadType,
    crypto::{
        keys::{Ed25519PrivateKey, Ed25519PublicKey, X25519PrivateKey},
        proofs::{
            PoQVerificationInputsMinusSigningKey,
            quota::{ProofOfQuota, inputs::prove::public::LeaderInputs},
            selection::{ProofOfSelection, inputs::VerifyInputs},
        },
        signatures::{SIGNATURE_SIZE, Signature},
    },
    encap::{
        ProofsVerifier, decapsulated::DecapsulationOutput, encapsulated::EncapsulatedMessage,
        validated::RequiredProofOfSelectionVerificationInputs,
    },
    input::{EncapsulationInput, EncapsulationInputs},
    message::payload::MAX_PAYLOAD_BODY_SIZE,
};

const ENCAPSULATION_COUNT: usize = 3;

struct NeverFailingProofsVerifier;

impl ProofsVerifier for NeverFailingProofsVerifier {
    type Error = Infallible;

    fn new(_public_inputs: PoQVerificationInputsMinusSigningKey) -> Self {
        Self
    }

    fn start_epoch_transition(&mut self, _new_pol_inputs: LeaderInputs) {}

    fn complete_epoch_transition(&mut self) {}

    fn verify_proof_of_quota(
        &self,
        _proof: ProofOfQuota,
        _signing_key: &Ed25519PublicKey,
    ) -> Result<ZkHash, Self::Error> {
        use groth16::Field as _;

        Ok(ZkHash::ZERO)
    }

    fn verify_proof_of_selection(
        &self,
        _proof: ProofOfSelection,
        _inputs: &VerifyInputs,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct AlwaysFailingProofOfQuotaVerifier;

impl ProofsVerifier for AlwaysFailingProofOfQuotaVerifier {
    type Error = ();

    fn new(_public_inputs: PoQVerificationInputsMinusSigningKey) -> Self {
        Self
    }

    fn start_epoch_transition(&mut self, _new_pol_inputs: LeaderInputs) {}

    fn complete_epoch_transition(&mut self) {}

    fn verify_proof_of_quota(
        &self,
        _proof: ProofOfQuota,
        _signing_key: &Ed25519PublicKey,
    ) -> Result<ZkHash, Self::Error> {
        Err(())
    }

    fn verify_proof_of_selection(
        &self,
        _proof: ProofOfSelection,
        _inputs: &VerifyInputs,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct AlwaysFailingProofOfSelectionVerifier;

impl ProofsVerifier for AlwaysFailingProofOfSelectionVerifier {
    type Error = ();

    fn new(_public_inputs: PoQVerificationInputsMinusSigningKey) -> Self {
        Self
    }

    fn start_epoch_transition(&mut self, _new_pol_inputs: LeaderInputs) {}

    fn complete_epoch_transition(&mut self) {}

    fn verify_proof_of_quota(
        &self,
        _proof: ProofOfQuota,
        _signing_key: &Ed25519PublicKey,
    ) -> Result<ZkHash, Self::Error> {
        use groth16::Field as _;

        Ok(ZkHash::ZERO)
    }

    fn verify_proof_of_selection(
        &self,
        _proof: ProofOfSelection,
        _inputs: &VerifyInputs,
    ) -> Result<(), Self::Error> {
        Err(())
    }
}

#[test]
fn encapsulate_and_decapsulate() {
    const PAYLOAD_BODY: &[u8] = b"hello";
    let verifier = NeverFailingProofsVerifier;

    let (inputs, blend_node_enc_keys) = generate_inputs(2).unwrap();
    let msg =
        EncapsulatedMessage::<ENCAPSULATION_COUNT>::new(&inputs, PayloadType::Data, PAYLOAD_BODY)
            .unwrap();

    // NOTE: We expect that the decapsulations can be done
    // in the "reverse" order of blend_node_enc_keys.
    // (following the notion in the spec)

    // We can decapsulate with the correct private key.
    let DecapsulationOutput::Incompleted(msg) = msg
        .verify_public_header(&verifier)
        .unwrap()
        .decapsulate(
            blend_node_enc_keys.last().unwrap(),
            &RequiredProofOfSelectionVerificationInputs::default(),
            &verifier,
        )
        .unwrap()
    else {
        panic!("Expected an incompleted message");
    };

    // We cannot decapsulate with an invalid private key,
    // which we already used for the first decapsulation.
    assert!(
        msg.clone()
            .verify_public_header(&verifier)
            .unwrap()
            .decapsulate(
                blend_node_enc_keys.last().unwrap(),
                &RequiredProofOfSelectionVerificationInputs::default(),
                &verifier,
            )
            .is_err()
    );

    // We can decapsulate with the correct private key
    // and the fully-decapsulated payload is correct.
    let DecapsulationOutput::Completed(decapsulated_message) = msg
        .verify_public_header(&verifier)
        .unwrap()
        .decapsulate(
            blend_node_enc_keys.first().unwrap(),
            &RequiredProofOfSelectionVerificationInputs::default(),
            &verifier,
        )
        .unwrap()
    else {
        panic!("Expected an incompleted message");
    };
    // The payload body should be the same as the original one.
    assert_eq!(decapsulated_message.payload_type(), PayloadType::Data);
    assert_eq!(decapsulated_message.payload_body(), PAYLOAD_BODY);
}

#[test]
fn payload_too_long() {
    let (inputs, _) = generate_inputs(1).unwrap();
    assert!(matches!(
        EncapsulatedMessage::<ENCAPSULATION_COUNT>::new(
            &inputs,
            PayloadType::Data,
            &vec![0u8; MAX_PAYLOAD_BODY_SIZE + 1]
        )
        .err(),
        Some(Error::PayloadTooLarge)
    ));
}

#[test]
fn invalid_public_header_signature() {
    const PAYLOAD_BODY: &[u8] = b"hello";
    let verifier = NeverFailingProofsVerifier;

    let msg_with_invalid_signature = {
        let (inputs, _) = generate_inputs(2).unwrap();
        let mut msg = EncapsulatedMessage::<ENCAPSULATION_COUNT>::new(
            &inputs,
            PayloadType::Data,
            PAYLOAD_BODY,
        )
        .unwrap();
        *msg.public_header_mut().signature_mut() = Signature::from([100u8; SIGNATURE_SIZE]);
        msg
    };

    let public_header_verification_result =
        msg_with_invalid_signature.verify_public_header(&verifier);
    assert!(matches!(
        public_header_verification_result,
        Err(Error::SignatureVerificationFailed)
    ));
}

#[test]
fn invalid_public_header_proof_of_quota() {
    use crate::crypto::proofs::quota::Error as PoQError;

    const PAYLOAD_BODY: &[u8] = b"hello";
    let verifier = AlwaysFailingProofOfQuotaVerifier;

    let (inputs, _) = generate_inputs(2).unwrap();
    let msg =
        EncapsulatedMessage::<ENCAPSULATION_COUNT>::new(&inputs, PayloadType::Data, PAYLOAD_BODY)
            .unwrap();

    let public_header_verification_result = msg.verify_public_header(&verifier);
    assert!(matches!(
        public_header_verification_result,
        Err(Error::ProofOfQuotaVerificationFailed(
            PoQError::InvalidProof
        ))
    ));
}

#[test]
fn invalid_blend_header_proof_of_selection() {
    use crate::crypto::proofs::selection::Error as PoSelError;

    const PAYLOAD_BODY: &[u8] = b"hello";
    let verifier = AlwaysFailingProofOfSelectionVerifier;

    let (inputs, blend_node_enc_keys) = generate_inputs(2).unwrap();
    let msg =
        EncapsulatedMessage::<ENCAPSULATION_COUNT>::new(&inputs, PayloadType::Data, PAYLOAD_BODY)
            .unwrap();

    let validated_message = msg.verify_public_header(&verifier).unwrap();

    let validated_message_decapsulation_result = validated_message.decapsulate(
        blend_node_enc_keys.last().unwrap(),
        &RequiredProofOfSelectionVerificationInputs::default(),
        &verifier,
    );
    assert!(matches!(
        validated_message_decapsulation_result,
        Err(Error::ProofOfSelectionVerificationFailed(
            PoSelError::Verification
        ))
    ));
}

fn generate_inputs(
    cnt: usize,
) -> Result<
    (
        EncapsulationInputs<ENCAPSULATION_COUNT>,
        Vec<X25519PrivateKey>,
    ),
    Error,
> {
    let recipient_signing_keys = core::iter::repeat_with(Ed25519PrivateKey::generate)
        .take(cnt)
        .collect::<Vec<_>>();
    let inputs = EncapsulationInputs::new(
        recipient_signing_keys
            .iter()
            .map(|recipient_signing_key| {
                EncapsulationInput::new(
                    Ed25519PrivateKey::generate(),
                    &recipient_signing_key.public_key(),
                    ProofOfQuota::dummy(),
                    ProofOfSelection::dummy(),
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )?;
    Ok((
        inputs,
        recipient_signing_keys
            .iter()
            .map(Ed25519PrivateKey::derive_x25519)
            .collect(),
    ))
}
