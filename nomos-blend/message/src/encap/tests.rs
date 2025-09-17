use core::convert::Infallible;

use nomos_core::crypto::ZkHash;

use crate::{
    crypto::{
        keys::{Ed25519PrivateKey, X25519PrivateKey},
        proofs::{
            quota::{inputs::prove::PublicInputs, ProofOfQuota},
            selection::{inputs::VerifyInputs, ProofOfSelection},
        },
    },
    encap::{
        decapsulated::DecapsulationOutput, encapsulated::EncapsulatedMessage,
        unwrapped::RequiredProofOfSelectionVerificationInputs, ProofsVerifier,
    },
    input::{EncapsulationInput, EncapsulationInputs},
    message::payload::MAX_PAYLOAD_BODY_SIZE,
    Error, PayloadType,
};

const ENCAPSULATION_COUNT: usize = 3;

struct NeverFailingProofsVerifier;

impl ProofsVerifier for NeverFailingProofsVerifier {
    type Error = Infallible;

    fn new() -> Self {
        NeverFailingProofsVerifier
    }

    fn verify_proof_of_quota(
        &self,
        _proof: ProofOfQuota,
        _inputs: &PublicInputs,
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
        .verify_and_unwrap_public_header(&PublicInputs::default(), &verifier)
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
    assert!(msg
        .clone()
        .verify_and_unwrap_public_header(&PublicInputs::default(), &verifier)
        .unwrap()
        .decapsulate(
            blend_node_enc_keys.last().unwrap(),
            &RequiredProofOfSelectionVerificationInputs::default(),
            &verifier,
        )
        .is_err());

    // We can decapsulate with the correct private key
    // and the fully-decapsulated payload is correct.
    let DecapsulationOutput::Completed(decapsulated_message) = msg
        .verify_and_unwrap_public_header(&PublicInputs::default(), &verifier)
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
    assert_eq!(
        EncapsulatedMessage::<ENCAPSULATION_COUNT>::new(
            &inputs,
            PayloadType::Data,
            &vec![0u8; MAX_PAYLOAD_BODY_SIZE + 1]
        )
        .err(),
        Some(Error::PayloadTooLarge)
    );
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
