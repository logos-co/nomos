use std::iter::repeat_n;

use itertools::Itertools as _;
use nomos_core::wire;
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

use crate::{
    crypto::{
        random_sized_bytes, Ed25519PrivateKey, Ed25519PublicKey, ProofOfQuota, ProofOfSelection,
        SharedKey, Signature, X25519PrivateKey,
    },
    error::Error,
    input::EncapsulationInputs,
    message::{BlendingHeader, Payload, PayloadType, PublicHeader},
};

pub type MessageIdentifier = Ed25519PublicKey;

/// An encapsulated message that is sent to the blend network.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncapsulatedMessage<const ENCAPSULATION_COUNT: usize> {
    /// A public header that is not encapsulated.
    public_header: PublicHeader,
    /// Encapsulated parts
    encapsulated_part: EncapsulatedPart<ENCAPSULATION_COUNT>,
}

impl<const ENCAPSULATION_COUNT: usize> EncapsulatedMessage<ENCAPSULATION_COUNT> {
    /// Creates a new [`EncapsulatedMessage`] with the provided inputs and
    /// payload.
    pub fn new(
        inputs: &EncapsulationInputs<ENCAPSULATION_COUNT>,
        payload_type: PayloadType,
        payload_body: &[u8],
    ) -> Result<Self, Error> {
        // Create the encapsulated part.
        let (part, signing_key, proof_of_quota) = inputs.iter().enumerate().fold(
            (
                // Start with an initialized encapsulated part,
                // a random signing key, and proof of quota.
                EncapsulatedPart::initialize(inputs, payload_type, payload_body)?,
                Ed25519PrivateKey::generate(),
                ProofOfQuota::from(random_sized_bytes()),
            ),
            |(part, signing_key, proof_of_quota), (i, input)| {
                (
                    part.encapsulate(
                        &input.shared_key,
                        &signing_key,
                        proof_of_quota,
                        input.proof_of_selection.clone(),
                        i == 0,
                    ),
                    input.signing_key.clone(),
                    input.proof_of_quota,
                )
            },
        );

        // Construct the public header.
        let public_header = PublicHeader::new(
            signing_key.public_key(),
            proof_of_quota,
            part.sign(&signing_key),
        );

        Ok(Self {
            public_header,
            encapsulated_part: part,
        })
    }

    /// Decapsulates the message using the provided key.
    ///
    /// If the provided key is eligible, returns the following:
    /// - [`DecapsulationOutput::Completed`] if the message was fully
    ///   decapsulated by this call.
    /// - [`DecapsulationOutput::Incompleted`] if the message is still
    ///   encapsulated.
    ///
    /// If not, [`Error::DeserializationFailed`] or
    /// [`Error::ProofOfSelectionVerificationFailed`] will be returned.
    pub fn decapsulate(
        self,
        private_key: &X25519PrivateKey,
    ) -> Result<DecapsulationOutput<ENCAPSULATION_COUNT>, Error> {
        // Derive the shared key.
        let shared_key =
            private_key.derive_shared_key(&self.public_header.signing_pubkey().derive_x25519());

        // Decapsulate the encapsulated part.
        match self.encapsulated_part.decapsulate(&shared_key)? {
            PartDecapsulationOutput::Incompleted((encapsulated_part, public_header)) => {
                Ok(DecapsulationOutput::Incompleted(Self {
                    public_header,
                    encapsulated_part,
                }))
            }
            PartDecapsulationOutput::Completed(payload) => {
                let (payload_type, payload_body) = payload.try_into_components()?;
                Ok(DecapsulationOutput::Completed(DecapsulatedMessage {
                    payload_type,
                    payload_body,
                }))
            }
        }
    }

    #[must_use]
    pub const fn public_header(&self) -> &PublicHeader {
        &self.public_header
    }

    #[must_use]
    pub const fn id(&self) -> MessageIdentifier {
        *self.public_header.signing_pubkey()
    }

    pub fn verify_public_header(&self) -> Result<(), Error> {
        self.public_header
            .verify_signature(&EncapsulatedPart::signing_body(
                &self.encapsulated_part.private_header,
                &self.encapsulated_part.payload,
            ))?;
        // TODO: Add proof of quota verification
        Ok(())
    }

    #[cfg(feature = "unsafe-test-functions")]
    #[must_use]
    pub const fn public_header_mut(&mut self) -> &mut PublicHeader {
        &mut self.public_header
    }
}

#[derive(Clone, Debug)]
pub struct DecapsulatedMessage {
    payload_type: PayloadType,
    payload_body: Vec<u8>,
}

impl DecapsulatedMessage {
    #[must_use]
    pub const fn payload_type(&self) -> PayloadType {
        self.payload_type
    }

    #[must_use]
    pub fn payload_body(&self) -> &[u8] {
        &self.payload_body
    }

    #[must_use]
    pub fn into_components(self) -> (PayloadType, Vec<u8>) {
        (self.payload_type, self.payload_body)
    }
}

/// The output of [`EncapsulatedMessage::decapsulate`]
#[expect(
    clippy::large_enum_variant,
    reason = "Size difference between variants is not too large (small ENCAPSULATION_COUNT)"
)]
#[derive(Clone, Debug)]
pub enum DecapsulationOutput<const ENCAPSULATION_COUNT: usize> {
    Incompleted(EncapsulatedMessage<ENCAPSULATION_COUNT>),
    Completed(DecapsulatedMessage),
}

/// Part of the message that should be encapsulated.
// TODO: Consider having `InitializedPart`
// that just finished the initialization step and doesn't have `decapsulate` method.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct EncapsulatedPart<const ENCAPSULATION_COUNT: usize> {
    private_header: EncapsulatedPrivateHeader<ENCAPSULATION_COUNT>,
    payload: EncapsulatedPayload,
}

impl<const ENCAPSULATION_COUNT: usize> EncapsulatedPart<ENCAPSULATION_COUNT> {
    /// Initializes the encapsulated part as preparation for actual
    /// encapsulations.
    fn initialize(
        inputs: &EncapsulationInputs<ENCAPSULATION_COUNT>,
        payload_type: PayloadType,
        payload_body: &[u8],
    ) -> Result<Self, Error> {
        Ok(Self {
            private_header: EncapsulatedPrivateHeader::initialize(inputs),
            payload: EncapsulatedPayload::initialize(&Payload::new(payload_type, payload_body)?),
        })
    }

    /// Add a layer of encapsulation.
    fn encapsulate(
        self,
        shared_key: &SharedKey,
        signing_key: &Ed25519PrivateKey,
        proof_of_quota: ProofOfQuota,
        proof_of_selection: ProofOfSelection,
        is_last: bool,
    ) -> Self {
        // Compute the signature of the current encapsulated part.
        let signature = self.sign(signing_key);

        // Encapsulate the private header.
        let private_header = self.private_header.encapsulate(
            shared_key,
            signing_key.public_key(),
            proof_of_quota,
            signature,
            proof_of_selection,
            is_last,
        );

        // Encrypt the payload.
        let payload = self.payload.encapsulate(shared_key);

        Self {
            private_header,
            payload,
        }
    }

    /// Decapsulate a layer.
    fn decapsulate(
        self,
        key: &SharedKey,
    ) -> Result<PartDecapsulationOutput<ENCAPSULATION_COUNT>, Error> {
        match self.private_header.decapsulate(key)? {
            PrivateHeaderDecapsulationOutput::Incompleted((private_header, public_header)) => {
                let payload = self.payload.decapsulate(key);
                Self::verify_reconstructed_public_header(
                    &public_header,
                    &private_header,
                    &payload,
                )?;
                Ok(PartDecapsulationOutput::Incompleted((
                    Self {
                        private_header,
                        payload,
                    },
                    public_header,
                )))
            }
            PrivateHeaderDecapsulationOutput::Completed((private_header, public_header)) => {
                let payload = self.payload.decapsulate(key);
                Self::verify_reconstructed_public_header(
                    &public_header,
                    &private_header,
                    &payload,
                )?;
                Ok(PartDecapsulationOutput::Completed(
                    payload.try_deserialize()?,
                ))
            }
        }
    }

    /// Signs the encapsulated part using the provided key.
    fn sign(&self, key: &Ed25519PrivateKey) -> Signature {
        key.sign(&Self::signing_body(&self.private_header, &self.payload))
    }

    /// Verify the public header reconstructed when decapsulating the private
    /// header.
    fn verify_reconstructed_public_header(
        public_header: &PublicHeader,
        private_header: &EncapsulatedPrivateHeader<ENCAPSULATION_COUNT>,
        payload: &EncapsulatedPayload,
    ) -> Result<(), Error> {
        // Verify the signature in the reconstructed public header
        public_header.verify_signature(&Self::signing_body(private_header, payload))
    }

    /// Returns the body that should be signed.
    fn signing_body(
        private_header: &EncapsulatedPrivateHeader<ENCAPSULATION_COUNT>,
        payload: &EncapsulatedPayload,
    ) -> Vec<u8> {
        private_header
            .iter_bytes()
            .chain(payload.iter_bytes())
            .collect::<Vec<_>>()
    }
}

/// The output of [`EncapsulatedPart::decapsulate`]
#[expect(
    clippy::large_enum_variant,
    reason = "Size difference between variants is not too large (small ENCAPSULATION_COUNT)"
)]
enum PartDecapsulationOutput<const ENCAPSULATION_COUNT: usize> {
    Incompleted((EncapsulatedPart<ENCAPSULATION_COUNT>, PublicHeader)),
    Completed(Payload),
}

/// An encapsulated private header, which is a set of encapsulated blending
/// headers.
// TODO: Consider having `InitializedPrivateHeader`
// that just finished the initialization step and doesn't have `decapsulate` method.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct EncapsulatedPrivateHeader<const ENCAPSULATION_COUNT: usize>(
    #[serde(with = "BigArray")] [EncapsulatedBlendingHeader; ENCAPSULATION_COUNT],
);

impl<const ENCAPSULATION_COUNT: usize> EncapsulatedPrivateHeader<ENCAPSULATION_COUNT> {
    /// Initializes the private header as preparation for actual encapsulations.
    fn initialize(inputs: &EncapsulationInputs<ENCAPSULATION_COUNT>) -> Self {
        // Randomize the private header in the reconstructable way,
        // so that the corresponding signatures can be verified later.
        // Plus, encapsulate the last `inputs.len()` blending headers.
        //
        // BlendingHeaders[0]:       random
        // BlendingHeaders[1]: inputs[1](inputs[0](pseudo_random(inputs[1])))
        // BlendingHeaders[2]:           inputs[0](pseudo_random(inputs[0])
        Self(
            inputs
                .iter()
                .map(|input| Some(&input.shared_key))
                .chain(repeat_n(None, inputs.num_empty_slots()))
                .rev()
                .map(|rng_key| {
                    rng_key.map_or_else(
                        || EncapsulatedBlendingHeader::initialize(&BlendingHeader::random()),
                        |rng_key| {
                            let mut header = EncapsulatedBlendingHeader::initialize(
                                &BlendingHeader::pseudo_random(rng_key.as_slice()),
                            );
                            // Encapsulate the blending header with the shared key of each input
                            // until the shared key equal to the `rng_key` is encountered
                            // (inclusive).
                            inputs
                                .iter()
                                .take_while_inclusive(|&input| &input.shared_key != rng_key)
                                .for_each(|input| {
                                    header.encapsulate(&input.shared_key);
                                });
                            header
                        },
                    )
                })
                .collect::<Vec<_>>()
                .try_into()
                .expect("Expected num of BlendingHeaders must be generated"),
        )
    }

    /// Encapsulates the private header.
    fn encapsulate(
        mut self,
        shared_key: &SharedKey,
        signing_pubkey: Ed25519PublicKey,
        proof_of_quota: ProofOfQuota,
        signature: Signature,
        proof_of_selection: ProofOfSelection,
        is_last: bool,
    ) -> Self {
        // Shift blending headers by one rightward.
        self.shift_right();

        // Replace the first blending header with the new one.
        self.replace_first(EncapsulatedBlendingHeader::initialize(&BlendingHeader {
            signing_pubkey,
            proof_of_quota,
            signature,
            proof_of_selection,
            is_last,
        }));

        // Encrypt all blending headers
        self.0.iter_mut().for_each(|header| {
            header.encapsulate(shared_key);
        });

        self
    }

    fn decapsulate(
        mut self,
        key: &SharedKey,
    ) -> Result<PrivateHeaderDecapsulationOutput<ENCAPSULATION_COUNT>, Error> {
        // Decrypt all blending headers
        self.0.iter_mut().for_each(|header| {
            header.decapsulate(key);
        });

        // Check if the first blending header which was correctly decrypted
        // by verifying the decrypted proof of selection.
        // If the `private_key` is not correct, the proof of selection is
        // badly decrypted and verification will fail.
        let first_blending_header = self.first().try_deserialize()?;
        if !first_blending_header.proof_of_selection.verify() {
            return Err(Error::ProofOfSelectionVerificationFailed);
        }

        // Build a new public header with the values in the first blending header.
        let public_header = PublicHeader::new(
            first_blending_header.signing_pubkey,
            first_blending_header.proof_of_quota,
            first_blending_header.signature,
        );

        // Shift blending headers one leftward.
        self.shift_left();

        // Reconstruct/encrypt the last blending header
        // in the same way as the initialization step.
        let mut last_blending_header =
            EncapsulatedBlendingHeader::initialize(&BlendingHeader::pseudo_random(key.as_slice()));
        last_blending_header.encapsulate(key);
        self.replace_last(last_blending_header);

        if first_blending_header.is_last {
            Ok(PrivateHeaderDecapsulationOutput::Completed((
                self,
                public_header,
            )))
        } else {
            Ok(PrivateHeaderDecapsulationOutput::Incompleted((
                self,
                public_header,
            )))
        }
    }

    fn shift_right(&mut self) {
        self.0.rotate_right(1);
    }

    fn shift_left(&mut self) {
        self.0.rotate_left(1);
    }

    const fn first(&self) -> &EncapsulatedBlendingHeader {
        self.0
            .first()
            .expect("private header always have ENCAPSULATION_COUNT blending headers")
    }

    fn replace_first(&mut self, header: EncapsulatedBlendingHeader) {
        *self
            .0
            .first_mut()
            .expect("private header always have ENCAPSULATION_COUNT blending headers") = header;
    }

    fn replace_last(&mut self, header: EncapsulatedBlendingHeader) {
        *self
            .0
            .last_mut()
            .expect("private header always have ENCAPSULATION_COUNT blending headers") = header;
    }

    fn iter_bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.0
            .iter()
            .flat_map(EncapsulatedBlendingHeader::iter_bytes)
    }
}

/// The output of [`EncapsulatedPrivateHeader::decapsulate`]
enum PrivateHeaderDecapsulationOutput<const ENCAPSULATION_COUNT: usize> {
    Incompleted((EncapsulatedPrivateHeader<ENCAPSULATION_COUNT>, PublicHeader)),
    Completed((EncapsulatedPrivateHeader<ENCAPSULATION_COUNT>, PublicHeader)),
}

/// A blending header encapsulated zero or more times.
// TODO: Consider having `SerializedBlendingHeader` (not encapsulated).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct EncapsulatedBlendingHeader(Vec<u8>);

impl EncapsulatedBlendingHeader {
    /// Build a [`EncapsulatedBlendingHeader`] by serializing a
    /// [`BlendingHeader`] without any encapsulation.
    fn initialize(header: &BlendingHeader) -> Self {
        Self(wire::serialize(header).expect("BlendingHeader should be able to be serialized"))
    }

    /// Try to deserialize into a [`BlendingHeader`].
    /// If there is no encapsulation left, and if the bytes are valid,
    /// the deserialization will succeed.
    fn try_deserialize(&self) -> Result<BlendingHeader, Error> {
        wire::deserialize(&self.0).map_err(|_| Error::DeserializationFailed)
    }

    /// Add a layer of encapsulation.
    fn encapsulate(&mut self, key: &SharedKey) {
        key.encrypt(self.0.as_mut_slice());
    }

    /// Remove a layer of encapsulation.
    fn decapsulate(&mut self, key: &SharedKey) {
        key.decrypt(self.0.as_mut_slice());
    }

    fn iter_bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.0.iter().copied()
    }
}

/// A payload encapsulated zero or more times.
// TODO: Consider having `SerializedPayload` (not encapsulated).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct EncapsulatedPayload(Vec<u8>);

impl EncapsulatedPayload {
    /// Build a [`EncapsulatedPayload`] by serializing a [`Payload`]
    /// without any encapsulation.
    fn initialize(payload: &Payload) -> Self {
        Self(wire::serialize(payload).expect("Payload should be able to be serialized"))
    }

    /// Try to deserialize into a [`Payload`].
    /// If there is no encapsulation left, and if the bytes are valid,
    /// the deserialization will succeed.
    fn try_deserialize(&self) -> Result<Payload, Error> {
        wire::deserialize(&self.0).map_err(|_| Error::DeserializationFailed)
    }

    /// Add a layer of encapsulation.
    fn encapsulate(mut self, key: &SharedKey) -> Self {
        key.encrypt(self.0.as_mut_slice());
        self
    }

    /// Remove a layer of encapsulation.
    fn decapsulate(mut self, key: &SharedKey) -> Self {
        key.decrypt(self.0.as_mut_slice());
        self
    }

    fn iter_bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.0.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{input::EncapsulationInput, message::MAX_PAYLOAD_BODY_SIZE};

    const ENCAPSULATION_COUNT: usize = 3;

    #[test]
    fn encapsulate_and_decapsulate() {
        const PAYLOAD_BODY: &[u8] = b"hello";

        let (inputs, blend_node_enc_keys) = generate_inputs(2).unwrap();
        let msg = EncapsulatedMessage::<ENCAPSULATION_COUNT>::new(
            &inputs,
            PayloadType::Data,
            PAYLOAD_BODY,
        )
        .unwrap();

        // NOTE: We expect that the decapsulations can be done
        // in the "reverse" order of blend_node_enc_keys.
        // (following the notion in the spec)

        // We can decapsulate with the correct private key.
        let DecapsulationOutput::Incompleted(msg) = msg
            .decapsulate(blend_node_enc_keys.last().unwrap())
            .unwrap()
        else {
            panic!("Expected an incompleted message");
        };

        // We cannot decapsulate with an invalid private key,
        // which we already used for the first decapsulation.
        assert!(msg
            .clone()
            .decapsulate(blend_node_enc_keys.last().unwrap())
            .is_err());

        // We can decapsulate with the correct private key
        // and the fully-decapsulated payload is correct.
        let DecapsulationOutput::Completed(DecapsulatedMessage {
            payload_type,
            payload_body,
        }) = msg
            .decapsulate(blend_node_enc_keys.first().unwrap())
            .unwrap()
        else {
            panic!("Expected an incompleted message");
        };
        // The payload body should be the same as the original one.
        assert_eq!(payload_type, PayloadType::Data);
        assert_eq!(payload_body, PAYLOAD_BODY);
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
        let recipient_signing_keys = std::iter::repeat_with(Ed25519PrivateKey::generate)
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
}
