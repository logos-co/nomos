use bytes::BytesMut;

use crate::{
    crypto::{
        pseudo_random_bytes, Ed25519PrivateKey, Ed25519PublicKey, ProofOfQuota, ProofOfSelection,
        SharedKey, X25519PrivateKey, PROOF_OF_QUOTA_SIZE,
    },
    view::{RegionView, ShiftDirection, View, MESSAGE_SIZE},
};

pub const VERSION: u8 = 1;
// TODO: Make these generic const parameters.
pub const MAX_ENCAPSULATIONS: usize = 3;
pub const MAX_PAYLOAD_BODY_SIZE: usize = 34 * 1024;

/// A wrapper of a fixed-size Blend message.
///
/// This treats the message in the form of a byte array of [`MESSAGE_SIZE`]
/// bytes and performs encapsulation and decapsulation in-place on the byte
/// array instead of implemeting serde, in order to minimize memory alocations
/// and copying, because of the following reasons:
/// - Message size is not small but fixed and known at compile time.
/// - Encapsulation/Decapsulation involves multiple memory manipulations, such
///   as encryption and shifting.
/// - Encryption/decryption should (can) be efficiently performed on the
///   multiple fields contiguous in memory.
/// - Only one node in the network can decapsulate the message. All other nodes
///   don't need to deserialize all fields that they will not be able to
///   decrypt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message(pub(crate) BytesMut);

impl Message {
    /// Creates a new [`Message`] from a [`BytesMut`] instance.
    /// The input must be exactly [`MESSAGE_SIZE`] bytes long.
    fn new(bytes: BytesMut) -> Result<Self, MessageError> {
        if bytes.len() != MESSAGE_SIZE {
            return Err(MessageError::InvalidMessageSize);
        }
        Ok(Self(bytes))
    }

    /// Encapsulates the given payload with the provided encapsulation inputs.
    /// Encryption inputs more than [`MAX_ENCAPSULATIONS`] are not allowed.
    pub fn encapsulate(
        inputs: &[EncapsulationInput],
        payload: &[u8],
    ) -> Result<Self, MessageError> {
        if inputs.len() > MAX_ENCAPSULATIONS {
            return Err(MessageError::MaxEncapsulationsExceeded);
        }
        Self::initialize(inputs, payload)?.encapsulate_all(inputs)
    }

    /// An initialization step defined in the spec to prepare encapsulations.
    fn initialize(
        inputs: &[EncapsulationInput],
        payload_body: &[u8],
    ) -> Result<Self, MessageError> {
        assert!(inputs.len() <= MAX_ENCAPSULATIONS);

        let mut msg = Self::new(BytesMut::zeroed(MESSAGE_SIZE))?;

        // Fill the header.
        let mut view = View::parse(&mut msg)?;
        view.header.fill(VERSION);

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
        //
        //          ...
        //          BlendingHeaders[k-3]
        // E2(E1(E0(BlendingHeaders[k-2])))
        //    E1(E0(BlendingHeaders[k-1]))
        //       E0(BlendingHeaders[k])
        let mut view = RegionView::new(&mut msg);
        let num_encapsulations = inputs.len();
        for (i, input) in inputs.iter().enumerate() {
            for j in 0..num_encapsulations - i {
                input.shared_key.encrypt(
                    view.blending_header_mut(j + MAX_ENCAPSULATIONS - num_encapsulations)?,
                );
            }
        }

        Ok(msg)
    }

    /// Encapsulates the message with all the provided inputs.
    fn encapsulate_all(self, inputs: &[EncapsulationInput]) -> Result<Self, MessageError> {
        assert!(inputs.len() <= MAX_ENCAPSULATIONS);

        let mut msg = self;

        // Encapsulate message using each shared key.
        for (i, input) in inputs.iter().enumerate() {
            let prev_input = if i == 0 { None } else { Some(&inputs[i - 1]) };
            msg = msg.encapsulate_once(input, prev_input)?;
        }

        // Construct the public header.
        let EncapsulationInput {
            signing_key,
            proof_of_quota,
            ..
        } = inputs.last().unwrap();
        let sig = signing_key.sign(RegionView::new(&mut msg).signing_body()?);
        View::parse(&mut msg)?.public_header.fill(
            signing_key.public_key().as_bytes(),
            proof_of_quota.as_bytes(),
            &sig,
        );

        Ok(msg)
    }

    /// Encapsulates the message once with a given input.
    ///
    /// A `prev_input` should be provided, if exists, to construct a blending
    /// header. If not, a random signing key and proof will be used.
    fn encapsulate_once(
        self,
        input: &EncapsulationInput,
        prev_input: Option<&EncapsulationInput>,
    ) -> Result<Self, MessageError> {
        let mut msg = self;

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
        let mut view = RegionView::new(&mut msg);
        let sig = signing_key.sign(view.signing_body()?);

        // Encrypt the payload using `input`.
        input.shared_key.encrypt(view.payload_mut()?);

        // Shift blending headers by one rightward.
        let mut view = View::parse(&mut msg)?;
        view.private_header.shift(ShiftDirection::Right);

        // Fill the first blending header.
        // The key, proof of quota, and signature of the `prev_input` should be used.
        // The proof of selection must be calculated using `input`.
        view.private_header.first().fill(
            signing_pubkey.as_bytes(),
            proof_of_quota.as_bytes(),
            &sig,
            input.proof_of_selection.as_bytes(),
        );

        // Encrypt the private header using `input`.
        let mut view = RegionView::new(&mut msg);
        for i in 0..MAX_ENCAPSULATIONS {
            input.shared_key.encrypt(view.blending_header_mut(i)?);
        }

        Ok(msg)
    }

    /// Decapsulates the message using the provided encryption private key.
    ///
    /// If the `private_key` is not the one for decapsulating the first
    /// encapsulation, [`MessageError::ProofOfSelectionVerificationFailed`]
    /// will be returned. If it is the correct key, the first encapsulation
    /// will be decapsulated.
    pub fn decapsulate(
        message: BytesMut,
        private_key: &X25519PrivateKey,
        // TODO: This should be replaced with a proper impl.
        verify_proof_of_selection: impl Fn(&ProofOfSelection) -> bool,
    ) -> Result<Self, MessageError> {
        let mut msg = Self::new(message)?;

        // Derive the shared key.
        let view = View::parse(&mut msg)?;
        let shared_key = private_key.derive_shared_key(
            &Ed25519PublicKey::from_slice(view.public_header.signing_pubkey)
                .map_err(|_| MessageError::InvalidPublicKey)?
                .derive_x25519(),
        );

        // Decrypt the private header.
        let mut view = RegionView::new(&mut msg);
        for i in 0..MAX_ENCAPSULATIONS {
            shared_key.decrypt(view.blending_header_mut(i)?);
        }

        // Check if the first blending header which was correctly decrypted
        // by verifying the decrypted proof of selection.
        // If the `private_key` is not correct, the proof of selection is
        // badly decrypted and verification will fail.
        let mut view = View::parse(&mut msg)?;
        if !verify_proof_of_selection(&view.private_header.first().proof_of_selection()) {
            return Err(MessageError::ProofOfSelectionVerificationFailed);
        }

        // Replace the public header with the values in the first blending header.
        let first_blending_header = view.private_header.first();
        view.public_header.fill(
            first_blending_header.signing_pubkey,
            first_blending_header.proof_of_quota,
            first_blending_header.signature,
        );

        // Decrypt the payload.
        let mut view = RegionView::new(&mut msg);
        shared_key.decrypt(view.payload_mut()?);

        // Shift blending headers one leftward.
        let mut view = View::parse(&mut msg)?;
        view.private_header.shift(ShiftDirection::Left);

        // Reconstruct the last blending header
        // in the same way as the initialization step.
        view.private_header
            .last()
            .fill_random(shared_key.as_slice());

        // Encrypt the reconstructed last blending header.
        let mut view = RegionView::new(&mut msg);
        shared_key.encrypt(view.blending_header_mut(MAX_ENCAPSULATIONS - 1)?);

        // TODO: Compare signatures (The spec should be clarified).

        Ok(msg)
    }
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

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum MessageError {
    #[error("Invalid message size")]
    InvalidMessageSize,
    #[error("Invalid message format")]
    InvalidMessageFormat,
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
    use crate::crypto::KEY_SIZE;

    #[test]
    fn encapsulate_and_decapsulate() {
        const PAYLOAD_BODY: &[u8] = b"hello";

        let (inputs, blend_node_signing_keys) = generate_inputs(2);
        let msg = Message::encapsulate(&inputs, PAYLOAD_BODY).unwrap();
        assert_eq!(msg.0.len(), MESSAGE_SIZE);

        // We expect that the decapsulations can be done
        // in the reverse order of blend_node_signing_keys.

        // We can decapsulate with the correct private key.
        let blend_node_signing_key = blend_node_signing_keys.last().unwrap();
        let msg = Message::decapsulate(
            msg.0,
            &blend_node_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == blend_node_signing_key.public_key().as_bytes()
            },
        )
        .unwrap();

        // We cannot decapsulate with an invalid private key,
        // which we already used for the first decapsulation.
        assert!(Message::decapsulate(
            msg.0.clone(),
            &blend_node_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == blend_node_signing_key.public_key().as_bytes()
            },
        )
        .is_err());

        // We can decapsulate with the correct private key
        // and the fully-decapsulated message is correct.
        let blend_node_signing_key = blend_node_signing_keys.first().unwrap();
        let mut msg = Message::decapsulate(
            msg.0,
            &blend_node_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == blend_node_signing_key.public_key().as_bytes()
            },
        )
        .unwrap();
        // The payload body should be the same as the original one.
        assert_eq!(
            View::parse(&mut msg).unwrap().payload.body().unwrap(),
            PAYLOAD_BODY
        );
    }

    #[test]
    fn max_encapsulations_exceeded() {
        let (inputs, _) = generate_inputs(MAX_ENCAPSULATIONS + 1);
        assert_eq!(
            Message::encapsulate(&inputs, b"hello"),
            Err(MessageError::MaxEncapsulationsExceeded)
        );
    }

    #[test]
    fn payload_too_long() {
        let (inputs, _) = generate_inputs(1);
        assert_eq!(
            Message::encapsulate(&inputs, &vec![0u8; MAX_PAYLOAD_BODY_SIZE + 1]),
            Err(MessageError::PayloadTooLarge)
        );
    }

    #[test]
    fn decapsulate_invalid_sized_message() {
        assert_eq!(
            Message::decapsulate(
                BytesMut::zeroed(MESSAGE_SIZE + 1),
                &X25519PrivateKey::from_bytes([0u8; KEY_SIZE]),
                |_| true
            ),
            Err(MessageError::InvalidMessageSize)
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
