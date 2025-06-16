use bytes::BytesMut;

use crate::crypto::{
    pseudo_random_bytes, Ed25519PrivateKey, Ed25519PublicKey, ProofOfQuota, ProofOfSelection,
    SharedKey, X25519PrivateKey, KEY_SIZE, PROOF_OF_QUOTA_SIZE, PROOF_OF_SELECTION_SIZE,
    SIGNATURE_SIZE,
};

const VERSION: u8 = 1;
const HEADER_SIZE: usize = 1;
const BLENDING_SUBHEADER_SIZE: usize = KEY_SIZE + SIGNATURE_SIZE + PROOF_OF_QUOTA_SIZE;
const BLENDING_HEADER_SIZE: usize = BLENDING_SUBHEADER_SIZE + PROOF_OF_SELECTION_SIZE;
const MAX_ENCAPSULATIONS: usize = 3;
const PUBLIC_HEADER_SIZE: usize = BLENDING_SUBHEADER_SIZE;
const PRIVATE_HEADER_SIZE: usize = BLENDING_HEADER_SIZE * MAX_ENCAPSULATIONS;
const PAYLOAD_HEADER_SIZE: usize = 1 + 2;
const MAX_PAYLOAD_BODY_SIZE: usize = 34 * 1024;
const PAYLOAD_SIZE: usize = PAYLOAD_HEADER_SIZE + MAX_PAYLOAD_BODY_SIZE;
const MESSAGE_SIZE: usize = HEADER_SIZE + PUBLIC_HEADER_SIZE + PRIVATE_HEADER_SIZE + PAYLOAD_SIZE;

#[derive(Debug, Clone)]
pub struct Message(BytesMut);

impl Message {
    pub fn new(
        encapsulation_inputs: &[EncapsulationInput],
        payload: &[u8],
    ) -> Result<Self, MessageError> {
        if encapsulation_inputs.len() > MAX_ENCAPSULATIONS {
            return Err(MessageError::MaxEncapsulationsExceeded);
        }
        Ok(Self::initialize(encapsulation_inputs).encapsulate(encapsulation_inputs, payload))
    }

    fn initialize(inputs: &[EncapsulationInput]) -> Self {
        assert!(inputs.len() <= MAX_ENCAPSULATIONS);

        let mut msg = BytesMut::zeroed(MESSAGE_SIZE);

        // Fill the message header.
        msg[0] = VERSION;

        // Randomize the private header.
        let private_header_pos = HEADER_SIZE + PUBLIC_HEADER_SIZE;
        for i in 0..MAX_ENCAPSULATIONS {
            let bytes = pseudo_random_bytes(
                Ed25519PrivateKey::generate().public_key().as_bytes(),
                BLENDING_HEADER_SIZE,
            );
            let offset = private_header_pos + i * BLENDING_HEADER_SIZE;
            msg[offset..offset + BLENDING_HEADER_SIZE].copy_from_slice(&bytes);
        }

        // Fill the last `shared_key.len()` blending headers with resonstructed data.
        let num_encapsulations = inputs.len();
        for (i, input) in inputs.iter().rev().enumerate() {
            let key = input.shared_key.as_slice();
            let r1 = pseudo_random_bytes(&concat(key, &[1]), KEY_SIZE);
            let r2 = pseudo_random_bytes(&concat(key, &[2]), PROOF_OF_QUOTA_SIZE);
            let r3 = pseudo_random_bytes(&concat(key, &[3]), SIGNATURE_SIZE);
            let r4 = pseudo_random_bytes(&concat(key, &[4]), PROOF_OF_SELECTION_SIZE);

            let blending_header_idx = i + MAX_ENCAPSULATIONS - num_encapsulations;

            let mut offset = private_header_pos + blending_header_idx * BLENDING_HEADER_SIZE;
            msg[offset..offset + r1.len()].copy_from_slice(&r1);
            offset += r1.len();
            msg[offset..offset + r2.len()].copy_from_slice(&r2);
            offset += r2.len();
            msg[offset..offset + r3.len()].copy_from_slice(&r3);
            offset += r3.len();
            msg[offset..offset + r4.len()].copy_from_slice(&r4);
        }

        // Encrypt the last `shared_key.len()` blending headers.
        for (i, input) in inputs.iter().enumerate() {
            let cipher = input.shared_key.cipher(BLENDING_HEADER_SIZE);
            for j in 0..num_encapsulations - i {
                let blending_header_idx = j + MAX_ENCAPSULATIONS - num_encapsulations;
                let offset = private_header_pos + blending_header_idx * BLENDING_HEADER_SIZE;
                cipher.encrypt(&mut msg[offset..offset + BLENDING_HEADER_SIZE]);
            }
        }

        Self(msg)
    }

    fn encapsulate(self, inputs: &[EncapsulationInput], payload: &[u8]) -> Self {
        assert!(inputs.len() <= MAX_ENCAPSULATIONS);

        let mut msg = self.0;

        let private_header_pos = HEADER_SIZE + PUBLIC_HEADER_SIZE;
        let payload_pos = private_header_pos + PRIVATE_HEADER_SIZE;
        // TODO: check payload size
        msg[payload_pos] = 0x01; // data message
        let payload_size: u16 = payload.len().try_into().unwrap();
        msg[payload_pos + 1..payload_pos + 3].copy_from_slice(&payload_size.to_le_bytes());
        msg[payload_pos + 3..payload_pos + 3 + payload.len()].copy_from_slice(payload);

        // Encapsulate the private header and payload using each shared key.
        for (i, input) in inputs.iter().enumerate() {
            // Sign on the concat of current private header and payload.
            let (signing_key, signing_pubkey, proof_of_quota) = if i == 0 {
                let initial_private_key = Ed25519PrivateKey::generate();
                let initial_public_key = initial_private_key.public_key();
                let initial_proof_of_quota = ProofOfQuota(
                    pseudo_random_bytes(initial_private_key.as_bytes(), PROOF_OF_QUOTA_SIZE)
                        .try_into()
                        .unwrap(),
                );
                (
                    initial_private_key,
                    initial_public_key,
                    initial_proof_of_quota,
                )
            } else {
                let EncapsulationInput {
                    signing_key,
                    proof_of_quota,
                    ..
                } = inputs.get(i - 1).unwrap();
                let signing_pubkey = signing_key.public_key();
                (signing_key.clone(), signing_pubkey, proof_of_quota.clone())
            };
            let sig = signing_key.sign(&msg[private_header_pos..]);

            // Encrypt the payload.
            input.shared_key.encrypt(&mut msg[payload_pos..]);

            // Shift blending headers by one rightward.
            msg.copy_within(
                private_header_pos..payload_pos - BLENDING_HEADER_SIZE,
                private_header_pos + BLENDING_HEADER_SIZE,
            );

            // Fill the first blending header.
            let mut pos = private_header_pos;
            msg[pos..pos + KEY_SIZE].copy_from_slice(signing_pubkey.as_bytes());
            pos += KEY_SIZE;
            msg[pos..pos + PROOF_OF_QUOTA_SIZE].copy_from_slice(proof_of_quota.as_bytes());
            pos += PROOF_OF_QUOTA_SIZE;
            msg[pos..pos + SIGNATURE_SIZE].copy_from_slice(&sig);
            pos += SIGNATURE_SIZE;
            msg[pos..pos + PROOF_OF_SELECTION_SIZE]
                .copy_from_slice(input.proof_of_selection.as_bytes());

            // Encrypt the private header.
            let cipher = input.shared_key.cipher(BLENDING_HEADER_SIZE);
            for i in 0..MAX_ENCAPSULATIONS {
                let offset = private_header_pos + i * BLENDING_HEADER_SIZE;
                cipher.encrypt(&mut msg[offset..offset + BLENDING_HEADER_SIZE]);
            }
        }

        // Construct the public header.
        let EncapsulationInput {
            signing_key,
            proof_of_quota,
            ..
        } = inputs.last().unwrap();
        let sig = signing_key.sign(&msg[private_header_pos..]);
        let mut pos = HEADER_SIZE;
        msg[pos..pos + KEY_SIZE].copy_from_slice(signing_key.public_key().as_bytes());
        pos += KEY_SIZE;
        msg[pos..pos + PROOF_OF_QUOTA_SIZE].copy_from_slice(proof_of_quota.as_bytes());
        pos += PROOF_OF_QUOTA_SIZE;
        msg[pos..pos + SIGNATURE_SIZE].copy_from_slice(&sig);

        Self(msg)
    }

    pub fn decapsulate(
        self,
        private_key: &X25519PrivateKey,
        verify_proof_of_selection: impl Fn(&ProofOfSelection) -> bool,
    ) -> Result<Self, MessageError> {
        let mut msg = self.0;

        // Derive the shared key.
        let recipient_public_key =
            Ed25519PublicKey::from_slice(&msg[HEADER_SIZE..HEADER_SIZE + KEY_SIZE])
                .map_err(|_| MessageError::InvalidPublicKey)?;
        let shared_key = private_key.derive_shared_key(&recipient_public_key.derive_x25519());

        // Decrypt the private header.
        let private_header_pos = HEADER_SIZE + PUBLIC_HEADER_SIZE;
        for i in 0..MAX_ENCAPSULATIONS {
            let offset = private_header_pos + i * BLENDING_HEADER_SIZE;
            shared_key.decrypt(&mut msg[offset..offset + BLENDING_HEADER_SIZE]);
        }

        // Verify the first blending header which was decrypted.
        let proof_of_selection = ProofOfSelection(
            msg[private_header_pos + KEY_SIZE + PROOF_OF_QUOTA_SIZE + SIGNATURE_SIZE
                ..private_header_pos + BLENDING_HEADER_SIZE]
                .try_into()
                .unwrap(),
        );
        if !verify_proof_of_selection(&proof_of_selection) {
            return Err(MessageError::ProofOfSelectionVerificationFailed);
        }
        // TODO: deduplication

        // Set the public header.
        let mut pos = HEADER_SIZE;
        msg.copy_within(private_header_pos..private_header_pos + KEY_SIZE, pos);
        pos += KEY_SIZE;
        msg.copy_within(
            private_header_pos + KEY_SIZE..private_header_pos + KEY_SIZE + PROOF_OF_QUOTA_SIZE,
            pos,
        );
        pos += PROOF_OF_QUOTA_SIZE;
        msg.copy_within(
            private_header_pos + KEY_SIZE + PROOF_OF_QUOTA_SIZE
                ..private_header_pos + KEY_SIZE + PROOF_OF_QUOTA_SIZE + SIGNATURE_SIZE,
            pos,
        );

        // Decrypt the payload.
        let payload_pos = private_header_pos + PRIVATE_HEADER_SIZE;
        shared_key.decrypt(&mut msg[payload_pos..]);

        // Shift blending headers one leftward.
        msg.copy_within(
            private_header_pos + BLENDING_HEADER_SIZE..private_header_pos + PRIVATE_HEADER_SIZE,
            private_header_pos,
        );

        // Reconstruct the last blending header.
        let key = shared_key.as_slice();
        let r1 = pseudo_random_bytes(&concat(key, &[1]), KEY_SIZE);
        let r2 = pseudo_random_bytes(&concat(key, &[2]), PROOF_OF_QUOTA_SIZE);
        let r3 = pseudo_random_bytes(&concat(key, &[3]), SIGNATURE_SIZE);
        let r4 = pseudo_random_bytes(&concat(key, &[4]), PROOF_OF_SELECTION_SIZE);
        let last_blending_header_pos =
            private_header_pos + (MAX_ENCAPSULATIONS - 1) * BLENDING_HEADER_SIZE;
        let mut offset = last_blending_header_pos;
        msg[offset..offset + r1.len()].copy_from_slice(&r1);
        offset += r1.len();
        msg[offset..offset + r2.len()].copy_from_slice(&r2);
        offset += r2.len();
        msg[offset..offset + r3.len()].copy_from_slice(&r3);
        offset += r3.len();
        msg[offset..offset + r4.len()].copy_from_slice(&r4);

        // Encrypt the reconstructed last blending header.
        shared_key.encrypt(
            &mut msg[last_blending_header_pos..last_blending_header_pos + BLENDING_HEADER_SIZE],
        );

        // TODO: Compare signatures

        Ok(Self(msg))
    }

    #[must_use]
    pub fn payload_body(&self) -> &[u8] {
        let payload_pos = HEADER_SIZE + PUBLIC_HEADER_SIZE + PRIVATE_HEADER_SIZE;
        let size = u16::from_le_bytes(self.0[payload_pos + 1..payload_pos + 3].try_into().unwrap())
            as usize;
        &self.0[payload_pos + PAYLOAD_HEADER_SIZE..payload_pos + PAYLOAD_HEADER_SIZE + size]
    }
}

pub struct EncapsulationInput {
    signing_key: Ed25519PrivateKey,
    shared_key: SharedKey,
    proof_of_quota: ProofOfQuota,
    proof_of_selection: ProofOfSelection,
}

impl EncapsulationInput {
    #[must_use]
    pub fn new(
        signing_key: Ed25519PrivateKey,
        recipient_signing_key: &Ed25519PublicKey,
        proof_of_quota: ProofOfQuota,
        proof_of_selection: ProofOfSelection,
    ) -> Self {
        let shared_key = signing_key
            .derive_x25519()
            .derive_shared_key(&recipient_signing_key.derive_x25519());
        Self {
            signing_key,
            shared_key,
            proof_of_quota,
            proof_of_selection,
        }
    }
}

#[derive(Debug)]
pub enum MessageError {
    MaxEncapsulationsExceeded,
    InvalidPublicKey,
    ProofOfSelectionVerificationFailed,
}

fn concat(a: &[u8], b: &[u8]) -> Vec<u8> {
    itertools::chain(a, b).copied().collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encapsulate_and_decapsulate() {
        const PAYLOAD_BODY: &[u8] = b"hello";

        let (inputs, recipient_signing_keys) = generate_inputs(2);
        let msg = Message::new(&inputs, PAYLOAD_BODY).unwrap();
        assert_eq!(msg.0.len(), MESSAGE_SIZE);

        // Cannot decapsulate with an invalid private key.
        let recipient_signing_key = recipient_signing_keys.first().unwrap();
        assert!(msg
            .clone()
            .decapsulate(
                &recipient_signing_key.derive_x25519(),
                |proof_of_selection| {
                    proof_of_selection.as_bytes() == recipient_signing_key.public_key().as_bytes()
                },
            )
            .is_err());

        // Can decapsulate with the correct private key.
        let recipient_signing_key = recipient_signing_keys.last().unwrap();
        let msg = msg
            .decapsulate(
                &recipient_signing_key.derive_x25519(),
                |proof_of_selection| {
                    proof_of_selection.as_bytes() == recipient_signing_key.public_key().as_bytes()
                },
            )
            .unwrap();

        // Can decapsulate with the correct private key
        // and the fully-decapsulated message is correct.
        let recipient_signing_key = recipient_signing_keys.first().unwrap();
        let msg = msg
            .decapsulate(
                &recipient_signing_key.derive_x25519(),
                |proof_of_selection| {
                    proof_of_selection.as_bytes() == recipient_signing_key.public_key().as_bytes()
                },
            )
            .unwrap();
        assert_eq!(msg.payload_body(), PAYLOAD_BODY);
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
