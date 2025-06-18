use std::iter::repeat_n;

use nomos_core::wire;
use serde::{Deserialize, Serialize};

use crate::{
    crypto::{
        pseudo_random_bytes, Ed25519PrivateKey, Ed25519PublicKey, ProofOfQuota, ProofOfSelection,
        SharedKey, Signature, X25519PrivateKey, KEY_SIZE, PROOF_OF_QUOTA_SIZE,
        PROOF_OF_SELECTION_SIZE, SIGNATURE_SIZE,
    },
    error::Error,
    message::{BlendingHeader, Header, Payload, PayloadHeader, PublicHeader},
};

// TODO: Consider having these as parameters or generics.
const MAX_ENCAPSULATIONS: usize = 3;
const MAX_PAYLOAD_BODY_SIZE: usize = 34 * 1024;

/// An encapsulated message that is sent to the blend network.
//
// TODO: Implement [`BlendMessage`] trait for this type.
#[derive(Clone, Serialize, Deserialize)]
pub struct EncapsulatedMessage {
    /// A header that is not encapsulated.
    header: Header,
    /// A public header that is not encapsulated.
    public_header: PublicHeader,
    /// An encapsulated private header.
    private_header: [Vec<u8>; MAX_ENCAPSULATIONS],
    /// A encapsulated payload.
    payload: Vec<u8>,
}

impl EncapsulatedMessage {
    /// Encapsulates the given payload with the provided encapsulation inputs.
    /// Encapsulation inputs more than [`MAX_ENCAPSULATIONS`] are not allowed.
    pub fn new(inputs: &[EncapsulationInput], payload: &[u8]) -> Result<Self, Error> {
        if inputs.len() > MAX_ENCAPSULATIONS {
            return Err(Error::MaxEncapsulationsExceeded);
        }
        if inputs.is_empty() {
            return Err(Error::EmptyEncapsulationInputs);
        }
        if payload.len() > MAX_PAYLOAD_BODY_SIZE {
            return Err(Error::PayloadTooLarge);
        }

        let msg = Self::initialize(inputs, payload)?;

        // Encapsulate message using each shared key.
        let (mut msg, _) = inputs.iter().fold((msg, None), |(msg, prev_input), input| {
            let msg = Self::encapsulate(msg, input, prev_input);
            (msg, Some(input))
        });

        // Construct the public header.
        let EncapsulationInput {
            signing_key,
            proof_of_quota,
            ..
        } = inputs.last().expect("inputs should not be empty");
        let sig = signing_key.sign(&msg.signing_body());
        msg.public_header = PublicHeader {
            signing_pubkey: signing_key.public_key(),
            proof_of_quota: proof_of_quota.clone(),
            signature: sig,
        };

        Ok(msg)
    }

    /// An initialization step for preparing encapsulations.
    fn initialize(inputs: &[EncapsulationInput], payload_body: &[u8]) -> Result<Self, Error> {
        // Fill the header.
        let header = Header::new();

        // Zero out the public header.
        let public_header = Self::zeroized_public_header();

        // Randomize the private header.
        // The last `inputs.len()` blending headers should be randomized with
        // resonstructed data. BlendingHeaders[0]: randomized
        // BlendingHeaders[1]: pseudo-randomized with inputs[1]
        // BlendingHeaders[2]: pseudo-randomized with inputs[0]
        assert!(inputs.len() <= MAX_ENCAPSULATIONS);
        let mut private_header: [Vec<u8>; MAX_ENCAPSULATIONS] = inputs
            .iter()
            .map(Some)
            .chain(repeat_n(None, MAX_ENCAPSULATIONS - inputs.len()))
            .rev()
            .map(|input| {
                input.map_or_else(Self::random_blending_header, |input| {
                    Self::pseudo_random_blending_header(input.shared_key.as_slice())
                })
            })
            .map(|blending_header| {
                wire::serialize(&blending_header).expect("BlendingHeader should be serialized")
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("Expected num of BlendingHeaders must be generated");

        // Encrypt the last `shared_key.len()` blending headers.
        // Each blending header should be encrypted in order of inputs.
        // Each subsequent blending header should be encrypted one layer less.
        //       BlendingHeaders[0]
        // E1(E0(BlendingHeaders[1])))
        //    E0(BlendingHeaders[2]))
        private_header
            .iter_mut()
            .rev()
            .take(inputs.len())
            .enumerate()
            .for_each(|(i, blending_header)| {
                inputs.iter().take(i + 1).for_each(|input| {
                    input.shared_key.encrypt(blending_header);
                });
            });

        // Fill the payload.
        let mut payload = Payload {
            header: PayloadHeader {
                payload_type: 0x01,
                body_len: payload_body
                    .len()
                    .try_into()
                    .map_err(|_| Error::InvalidPayloadLength)?,
            },
            body: vec![0u8; MAX_PAYLOAD_BODY_SIZE],
        };
        assert!(payload_body.len() <= payload.body.len());
        payload.body[..payload_body.len()].copy_from_slice(payload_body);
        let payload = wire::serialize(&payload).expect("Payload should be serialized");

        Ok(Self {
            header,
            public_header,
            private_header,
            payload,
        })
    }

    /// Encapsulates the message once with a given input.
    ///
    /// A `prev_input` should be provided, if exists, to construct a blending
    /// header. If not, a random signing key and proof will be used.
    fn encapsulate(
        mut self,
        input: &EncapsulationInput,
        prev_input: Option<&EncapsulationInput>,
    ) -> Self {
        // Generate a signature over the concatenation of the current private header and
        // payload, using the signing key of the `prev_input`.
        let (signing_key, signing_pubkey, proof_of_quota) = if let Some(EncapsulationInput {
            signing_key,
            proof_of_quota,
            ..
        }) = prev_input
        {
            let signing_pubkey = signing_key.public_key();
            (signing_key.clone(), signing_pubkey, proof_of_quota.clone())
        } else {
            // If `prev_input` is `None`, use a random key and proof.
            let signing_key = Ed25519PrivateKey::generate();
            let signing_pubkey = signing_key.public_key();
            let proof_of_quota = ProofOfQuota(
                pseudo_random_bytes(signing_key.as_bytes(), PROOF_OF_QUOTA_SIZE)
                    .try_into()
                    .unwrap(),
            );
            (signing_key, signing_pubkey, proof_of_quota)
        };
        let signature = signing_key.sign(&self.signing_body());

        // Encrypt the payload using `input`.
        input.shared_key.encrypt(&mut self.payload);

        // Shift blending headers by one rightward.
        self.private_header.rotate_right(1);

        // Fill the first blending header.
        // The key, proof of quota, and signature of the `prev_input` should be used.
        // The proof of selection must be calculated using `input`.
        let first_blending_header = self
            .private_header
            .first_mut()
            .expect("Private header shouldn't be empty");
        *first_blending_header = wire::serialize(&BlendingHeader {
            signing_pubkey,
            proof_of_quota,
            signature,
            proof_of_selection: input.proof_of_selection.clone(),
        })
        .unwrap();

        // Encrypt the private header using `input`.
        self.private_header.iter_mut().for_each(|header| {
            input.shared_key.encrypt(header);
        });

        self
    }

    /// Decapsulates the message using the provided encryption private key.
    ///
    /// If the `private_key` is not the one for decapsulating the first
    /// encapsulation, [`Error::ProofOfSelectionVerificationFailed`]
    /// will be returned. If it is the correct key, the first encapsulation
    /// will be decapsulated.
    pub fn decapsulate(
        mut msg: Self,
        private_key: &X25519PrivateKey,
        // TODO: Replace this with a proper impl.
        verify_proof_of_selection: impl Fn(&ProofOfSelection) -> bool,
    ) -> Result<Self, Error> {
        // Derive the shared key.
        let shared_key =
            private_key.derive_shared_key(&msg.public_header.signing_pubkey.derive_x25519());

        // Decrypt the private header.
        msg.private_header.iter_mut().for_each(|header| {
            shared_key.decrypt(header);
        });

        // Check if the first blending header which was correctly decrypted
        // by verifying the decrypted proof of selection.
        // If the `private_key` is not correct, the proof of selection is
        // badly decrypted and verification will fail.
        let first_blending_header: BlendingHeader =
            wire::deserialize(&msg.private_header[0]).map_err(|_| Error::InvalidMessageFormat)?;
        if !verify_proof_of_selection(&first_blending_header.proof_of_selection) {
            return Err(Error::ProofOfSelectionVerificationFailed);
        }

        // Replace the public header with the values in the first blending header.
        msg.public_header = PublicHeader {
            signing_pubkey: first_blending_header.signing_pubkey.clone(),
            proof_of_quota: first_blending_header.proof_of_quota.clone(),
            signature: first_blending_header.signature,
        };

        // Decrypt the payload.
        shared_key.decrypt(&mut msg.payload);

        // Shift blending headers one leftward.
        msg.private_header.rotate_left(1);

        // Reconstruct the last blending header
        // in the same way as the initialization step.
        *msg.private_header.last_mut().unwrap() =
            wire::serialize(&Self::pseudo_random_blending_header(shared_key.as_slice())).unwrap();

        // Encrypt the reconstructed last blending header.
        shared_key.encrypt(msg.private_header.last_mut().unwrap());

        // TODO: Verify the signature of the new public header.
        // The spec should be clarified on this.

        Ok(msg)
    }

    pub fn deserialize_payload(&self) -> Result<Payload, Error> {
        wire::deserialize(&self.payload).map_err(|_| Error::InvalidMessageFormat)
    }

    fn signing_body(&self) -> Vec<u8> {
        itertools::chain(
            self.private_header.iter().flat_map(|h| h.iter()),
            self.payload.iter(),
        )
        .copied()
        .collect()
    }

    fn zeroized_public_header() -> PublicHeader {
        PublicHeader {
            signing_pubkey: Ed25519PrivateKey::from_bytes(&[0u8; KEY_SIZE]).public_key(),
            proof_of_quota: ProofOfQuota::from_bytes(&[0u8; PROOF_OF_QUOTA_SIZE]),
            signature: Signature::from_bytes(&[0u8; SIGNATURE_SIZE]),
        }
    }

    /// Build a blending header with random data based on the provided key.
    /// in the reconstructable way.
    /// Each field in the header is filled with pseudo-random bytes derived from
    /// the key concatenated with a unique byte (1, 2, 3, or 4).
    fn pseudo_random_blending_header(key: &[u8]) -> BlendingHeader {
        // Unlike the spec, use random bytes to derive a private key
        // and derive the public key from it
        // because a public key cannot always be successfully derived from random bytes.
        //
        // TODO: This will be changed once we have zerocopy serde.
        let signing_pubkey = Ed25519PrivateKey::from_bytes(
            &pseudo_random_bytes(&concat(key, &[1]), KEY_SIZE)
                .as_slice()
                .try_into()
                .expect("Random bytes size should be correct"),
        )
        .public_key();
        let proof_of_quota = ProofOfQuota::from_bytes(
            &pseudo_random_bytes(&concat(key, &[2]), PROOF_OF_QUOTA_SIZE)
                .as_slice()
                .try_into()
                .expect("Random bytes size should be correct"),
        );
        let signature = Signature::from_bytes(
            &pseudo_random_bytes(&concat(key, &[3]), SIGNATURE_SIZE)
                .as_slice()
                .try_into()
                .expect("Random bytes size should be correct"),
        );
        let proof_of_selection = ProofOfSelection::from_bytes(
            &pseudo_random_bytes(&concat(key, &[4]), PROOF_OF_SELECTION_SIZE)
                .as_slice()
                .try_into()
                .expect("Random bytes size should be correct"),
        );
        BlendingHeader {
            signing_pubkey,
            proof_of_quota,
            signature,
            proof_of_selection,
        }
    }

    /// Build a blending header with random data based on the provided key.
    fn random_blending_header() -> BlendingHeader {
        // Use a zeroed key because this randomized blending header doesn't need to be
        // reconstructed.
        Self::pseudo_random_blending_header(&[0u8; KEY_SIZE])
    }
}

fn concat(a: &[u8], b: &[u8]) -> Vec<u8> {
    itertools::chain(a, b).copied().collect::<Vec<_>>()
}

/// Input for a single encapsulation,
pub struct EncapsulationInput {
    /// An ephemeral signing key generated by the encapsulating node.
    signing_key: Ed25519PrivateKey,
    /// A shared key derived from the ephemeral encryption key and
    /// the selected blend node's non-ephemeral encryption key.
    /// Encryption keys are derived from the signing keys.
    shared_key: SharedKey,
    /// A proof of quota for the encapsulation.
    proof_of_quota: ProofOfQuota,
    /// A proof of selection of the selected blend node.
    proof_of_selection: ProofOfSelection,
}

impl EncapsulationInput {
    /// Creates a new [`EncapsulationInput`]
    ///
    /// To derive the shared key, the `signing_key` and `blend_node_signing_key`
    /// are converted into encryptions keys.
    #[must_use]
    pub fn new(
        signing_key: Ed25519PrivateKey,
        blend_node_signing_key: &Ed25519PublicKey,
        proof_of_quota: ProofOfQuota,
        proof_of_selection: ProofOfSelection,
    ) -> Self {
        let shared_key = signing_key
            .derive_x25519()
            .derive_shared_key(&blend_node_signing_key.derive_x25519());
        Self {
            signing_key,
            shared_key,
            proof_of_quota,
            proof_of_selection,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encapsulate_and_decapsulate() {
        const PAYLOAD_BODY: &[u8] = b"hello";

        let (inputs, blend_node_signing_keys) = generate_inputs(2);
        let msg = EncapsulatedMessage::new(&inputs, PAYLOAD_BODY).unwrap();

        // We expect that the decapsulations can be done
        // in the reverse order of blend_node_signing_keys.

        // We can decapsulate with the correct private key.
        let blend_node_signing_key = blend_node_signing_keys.last().unwrap();
        let msg = EncapsulatedMessage::decapsulate(
            msg,
            &blend_node_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == blend_node_signing_key.public_key().as_bytes()
            },
        )
        .unwrap();

        // We cannot decapsulate with an invalid private key,
        // which we already used for the first decapsulation.
        assert!(EncapsulatedMessage::decapsulate(
            msg.clone(),
            &blend_node_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == blend_node_signing_key.public_key().as_bytes()
            },
        )
        .is_err());

        // We can decapsulate with the correct private key
        // and the fully-decapsulated message is correct.
        let blend_node_signing_key = blend_node_signing_keys.first().unwrap();
        let msg = EncapsulatedMessage::decapsulate(
            msg,
            &blend_node_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == blend_node_signing_key.public_key().as_bytes()
            },
        )
        .unwrap();
        // The payload body should be the same as the original one.
        assert_eq!(
            msg.deserialize_payload().unwrap().body().unwrap(),
            PAYLOAD_BODY
        );
    }

    #[test]
    fn max_encapsulations_exceeded() {
        let (inputs, _) = generate_inputs(MAX_ENCAPSULATIONS + 1);
        assert_eq!(
            EncapsulatedMessage::new(&inputs, b"hello").err(),
            Some(Error::MaxEncapsulationsExceeded)
        );
    }

    #[test]
    fn payload_too_long() {
        let (inputs, _) = generate_inputs(1);
        assert_eq!(
            EncapsulatedMessage::new(&inputs, &vec![0u8; MAX_PAYLOAD_BODY_SIZE + 1]).err(),
            Some(Error::PayloadTooLarge)
        );
    }

    fn generate_inputs(cnt: usize) -> (Vec<EncapsulationInput>, Vec<Ed25519PrivateKey>) {
        let recipient_signing_keys = std::iter::repeat_with(Ed25519PrivateKey::generate)
            .take(cnt)
            .collect::<Vec<_>>();
        let inputs = recipient_signing_keys
            .iter()
            .map(|recipient_signing_key| {
                let recipient_signing_pubkey = recipient_signing_key.public_key();
                let proof_of_selection = ProofOfSelection(*recipient_signing_pubkey.as_bytes());
                EncapsulationInput::new(
                    Ed25519PrivateKey::generate(),
                    &recipient_signing_pubkey,
                    ProofOfQuota([0u8; PROOF_OF_QUOTA_SIZE]),
                    proof_of_selection,
                )
            })
            .collect::<Vec<_>>();
        (inputs, recipient_signing_keys)
    }
}
