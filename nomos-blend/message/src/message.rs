use bytes::BytesMut;

use crate::{
    crypto::{
        pseudo_random_bytes, Ed25519PrivateKey, Ed25519PublicKey, ProofOfQuota, ProofOfSelection,
        SharedKey, X25519PrivateKey, PROOF_OF_QUOTA_SIZE,
    },
    view::{RegionView, ShiftDirection, View, MESSAGE_SIZE},
};

pub(crate) const VERSION: u8 = 1;
pub(crate) const MAX_ENCAPSULATIONS: usize = 3;
pub(crate) const MAX_PAYLOAD_BODY_SIZE: usize = 34 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message(pub(crate) BytesMut);

impl Message {
    pub fn encapsulate(
        encapsulation_inputs: &[EncapsulationInput],
        payload: &[u8],
    ) -> Result<Self, MessageError> {
        if encapsulation_inputs.len() > MAX_ENCAPSULATIONS {
            return Err(MessageError::MaxEncapsulationsExceeded);
        }
        Ok(Self::initialize(encapsulation_inputs, payload)?.encapsulate_all(encapsulation_inputs))
    }

    fn initialize(
        inputs: &[EncapsulationInput],
        payload_body: &[u8],
    ) -> Result<Self, MessageError> {
        assert!(inputs.len() <= MAX_ENCAPSULATIONS);

        let mut msg = Self(BytesMut::zeroed(MESSAGE_SIZE));

        // Fill the message header.
        let mut view = View::new(&mut msg);
        *view.header.version = VERSION;

        // Randomize the private header.
        for i in 0..MAX_ENCAPSULATIONS {
            view.private_header
                .get(i)
                .expect("BlendingHeader should exist")
                .fill_random(Ed25519PrivateKey::generate().public_key().as_bytes());
        }

        // Fill the last `inputs.len()` blending headers with resonstructed data.
        let num_encapsulations = inputs.len();
        for (i, input) in inputs.iter().rev().enumerate() {
            let blending_header = view
                .private_header
                .get(i + MAX_ENCAPSULATIONS - num_encapsulations)
                .expect("BlendingHeader should exist");
            blending_header.fill_random(input.shared_key.as_slice());
        }

        // Fill the payload.
        view.payload.fill(0x01, payload_body)?;

        // Encrypt the last `shared_key.len()` blending headers.
        // Example:
        //          ...
        //          BlendingHeaders[k-3]
        // E2(E1(E0(BlendingHeaders[k-2])))
        //    E1(E0(BlendingHeaders[k-1]))
        //       E0(BlendingHeaders[k])
        let mut view = RegionView::new(&mut msg);
        let num_encapsulations = inputs.len();
        for (i, input) in inputs.iter().enumerate() {
            for j in 0..num_encapsulations - i {
                input
                    .shared_key
                    .encrypt(view.blending_header_mut(j + MAX_ENCAPSULATIONS - num_encapsulations));
            }
        }

        Ok(msg)
    }

    fn encapsulate_all(self, inputs: &[EncapsulationInput]) -> Self {
        assert!(inputs.len() <= MAX_ENCAPSULATIONS);

        let mut msg = self;

        // Encapsulate the private header and payload using each shared key.
        for (i, input) in inputs.iter().enumerate() {
            msg = msg.encapsulate_once(input, if i == 0 { None } else { Some(&inputs[i - 1]) });
        }

        // Construct the public header.
        let EncapsulationInput {
            signing_key,
            proof_of_quota,
            ..
        } = inputs.last().unwrap();
        let sig = signing_key.sign(RegionView::new(&mut msg).signing_body());
        View::new(&mut msg).public_header.fill(
            signing_key.public_key().as_bytes(),
            proof_of_quota.as_bytes(),
            &sig,
        );

        msg
    }

    fn encapsulate_once(
        self,
        input: &EncapsulationInput,
        prev_input: Option<&EncapsulationInput>,
    ) -> Self {
        let mut msg = self;

        // Sign on the concat of current private header and payload.
        let (signing_key, signing_pubkey, proof_of_quota) = if let Some(EncapsulationInput {
            signing_key,
            proof_of_quota,
            ..
        }) = prev_input
        {
            let signing_pubkey = signing_key.public_key();
            (signing_key.clone(), signing_pubkey, proof_of_quota.clone())
        } else {
            // Generate a new signing key and proof of quota.
            let signing_key = Ed25519PrivateKey::generate();
            let signing_pubkey = signing_key.public_key();
            let proof_of_quota = ProofOfQuota(
                pseudo_random_bytes(signing_key.as_bytes(), PROOF_OF_QUOTA_SIZE)
                    .try_into()
                    .unwrap(),
            );
            (signing_key, signing_pubkey, proof_of_quota)
        };
        let mut view = RegionView::new(&mut msg);
        let sig = signing_key.sign(view.signing_body());

        // Encrypt the payload.
        input.shared_key.encrypt(view.payload_mut());

        // Shift blending headers by one rightward.
        let mut view = View::new(&mut msg);
        view.private_header.shift(ShiftDirection::Right);

        // Fill the first blending header.
        view.private_header.first().fill(
            signing_pubkey.as_bytes(),
            proof_of_quota.as_bytes(),
            &sig,
            input.proof_of_selection.as_bytes(),
        );

        // Encrypt the private header.
        let mut view = RegionView::new(&mut msg);
        for i in 0..MAX_ENCAPSULATIONS {
            input.shared_key.encrypt(view.blending_header_mut(i));
        }

        msg
    }

    pub fn decapsulate(
        message: BytesMut,
        private_key: &X25519PrivateKey,
        // TODO: This should be replaced with a proper impl.
        verify_proof_of_selection: impl Fn(&ProofOfSelection) -> bool,
    ) -> Result<Self, MessageError> {
        let mut msg = Self(message);

        // Derive the shared key.
        let view = View::new(&mut msg);
        let shared_key = private_key.derive_shared_key(
            &Ed25519PublicKey::from_slice(view.public_header.signing_pubkey)
                .map_err(|_| MessageError::InvalidPublicKey)?
                .derive_x25519(),
        );

        // Decrypt the private header.
        let mut view = RegionView::new(&mut msg);
        for i in 0..MAX_ENCAPSULATIONS {
            shared_key.decrypt(view.blending_header_mut(i));
        }

        // Verify the first blending header which was decrypted.
        let mut view = View::new(&mut msg);
        if !verify_proof_of_selection(&view.private_header.first().proof_of_selection()) {
            return Err(MessageError::ProofOfSelectionVerificationFailed);
        }

        // Set the public header.
        let first_blending_header = view.private_header.first();
        view.public_header.fill(
            first_blending_header.signing_pubkey,
            first_blending_header.proof_of_quota,
            first_blending_header.signature,
        );

        // Decrypt the payload.
        let mut view = RegionView::new(&mut msg);
        shared_key.decrypt(view.payload_mut());

        // Shift blending headers one leftward.
        let mut view = View::new(&mut msg);
        view.private_header.shift(ShiftDirection::Left);

        // Reconstruct the last blending header.
        view.private_header
            .last()
            .fill_random(shared_key.as_slice());

        // Encrypt the reconstructed last blending header.
        let mut view = RegionView::new(&mut msg);
        shared_key.encrypt(view.blending_header_mut(MAX_ENCAPSULATIONS - 1));

        // TODO: Compare signatures

        Ok(msg)
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

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum MessageError {
    #[error("Max encapsulations exceeded")]
    MaxEncapsulationsExceeded,
    #[error("Payload too large")]
    PayloadTooLarge,
    #[error("Proof of selection verification failed")]
    ProofOfSelectionVerificationFailed,
    #[error("Invalid public key")]
    InvalidPublicKey,
    #[error("Invalid payload length")]
    InvalidPayloadLen,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encapsulate_and_decapsulate() {
        const PAYLOAD_BODY: &[u8] = b"hello";

        let (inputs, recipient_signing_keys) = generate_inputs(2);
        let msg = Message::encapsulate(&inputs, PAYLOAD_BODY).unwrap();
        assert_eq!(msg.0.len(), MESSAGE_SIZE);

        // Cannot decapsulate with an invalid private key.
        let recipient_signing_key = recipient_signing_keys.first().unwrap();
        assert!(Message::decapsulate(
            msg.0.clone(),
            &recipient_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == recipient_signing_key.public_key().as_bytes()
            },
        )
        .is_err());

        // Can decapsulate with the correct private key.
        let recipient_signing_key = recipient_signing_keys.last().unwrap();
        let msg = Message::decapsulate(
            msg.0,
            &recipient_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == recipient_signing_key.public_key().as_bytes()
            },
        )
        .unwrap();

        // Can decapsulate with the correct private key
        // and the fully-decapsulated message is correct.
        let recipient_signing_key = recipient_signing_keys.first().unwrap();
        let mut msg = Message::decapsulate(
            msg.0,
            &recipient_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == recipient_signing_key.public_key().as_bytes()
            },
        )
        .unwrap();
        assert_eq!(View::new(&mut msg).payload.body().unwrap(), PAYLOAD_BODY);
    }

    #[test]
    fn too_long_payload() {
        const PAYLOAD_BODY: &[u8] = &[0u8; MAX_PAYLOAD_BODY_SIZE + 1];

        let (inputs, _) = generate_inputs(1);
        assert_eq!(
            Message::encapsulate(&inputs, PAYLOAD_BODY),
            Err(MessageError::PayloadTooLarge)
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
