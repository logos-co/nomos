use crate::{
    crypto::{
        keys::{Ed25519PrivateKey, X25519PrivateKey},
        proofs::{
            quota::{inputs::prove::PublicInputs, ProofOfQuota},
            selection::ProofOfSelection,
        },
    },
    encap::{
        decapsulated::DecapsulationOutput,
        encapsulated::{EncapsulatedMessage, PoSelVerificationInputs},
    },
    input::{EncapsulationInput, EncapsulationInputs},
    message::payload::MAX_PAYLOAD_BODY_SIZE,
    Error, PayloadType,
};

const ENCAPSULATION_COUNT: usize = 3;

#[test]
fn encapsulate_and_decapsulate() {
    const PAYLOAD_BODY: &[u8] = b"hello";

    let (inputs, blend_node_enc_keys) = generate_inputs(2).unwrap();
    let msg =
        EncapsulatedMessage::<ENCAPSULATION_COUNT>::new(&inputs, PayloadType::Data, PAYLOAD_BODY)
            .unwrap();

    // NOTE: We expect that the decapsulations can be done
    // in the "reverse" order of blend_node_enc_keys.
    // (following the notion in the spec)

    // We can decapsulate with the correct private key.
    let DecapsulationOutput::Incompleted(msg) = msg
        .verify_and_unwrap_public_header(PublicInputs::default())
        .unwrap()
        .decapsulate(
            blend_node_enc_keys.last().unwrap(),
            PoSelVerificationInputs::default(),
        )
        .unwrap()
    else {
        panic!("Expected an incompleted message");
    };

    // We cannot decapsulate with an invalid private key,
    // which we already used for the first decapsulation.
    assert!(msg
        .clone()
        .verify_and_unwrap_public_header(PublicInputs::default())
        .is_err());

    // We can decapsulate with the correct private key
    // and the fully-decapsulated payload is correct.
    let DecapsulationOutput::Completed(decapsulated_message) = msg
        .verify_and_unwrap_public_header(PublicInputs::default())
        .unwrap()
        .decapsulate(
            blend_node_enc_keys.first().unwrap(),
            PoSelVerificationInputs::default(),
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
                    ProofOfQuota::always_valid(),
                    ProofOfSelection::always_valid(),
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
