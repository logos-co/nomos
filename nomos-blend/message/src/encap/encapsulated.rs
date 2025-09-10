use core::iter::repeat_n;

use itertools::Itertools as _;
use nomos_core::wire;
use serde::{Deserialize, Serialize};

use crate::{
    crypto::{
        keys::{Ed25519PrivateKey, Ed25519PublicKey, SharedKey},
        proofs::{
            quota::{inputs::prove::PublicInputs, ProofOfQuota},
            selection::ProofOfSelection,
        },
        random_sized_bytes,
        signatures::Signature,
    },
    encap::{
        decapsulated::{PartDecapsulationOutput, PrivateHeaderDecapsulationOutput},
        unwrapped::UnwrappedEncapsulatedMessage,
    },
    input::EncapsulationInputs,
    message::{BlendingHeader, Payload, PublicHeader},
    Error, PayloadType,
};

pub type MessageIdentifier = Ed25519PublicKey;

/// An encapsulated message that is sent to the blend network.
#[derive(Clone, Serialize, Deserialize)]
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
                        input.ephemeral_encryption_key(),
                        &signing_key,
                        &proof_of_quota,
                        *input.proof_of_selection(),
                        i == 0,
                    ),
                    input.ephemeral_signing_key().clone(),
                    *input.proof_of_quota(),
                )
            },
        );

        // Construct the public header.
        let public_header = PublicHeader::new(
            signing_key.public_key(),
            &proof_of_quota,
            part.sign(&signing_key),
        );

        Ok(Self {
            public_header,
            encapsulated_part: part,
        })
    }

    #[must_use]
    pub const fn from_components(
        public_header: PublicHeader,
        encapsulated_part: EncapsulatedPart<ENCAPSULATION_COUNT>,
    ) -> Self {
        Self {
            public_header,
            encapsulated_part,
        }
    }

    pub fn verify_and_unwrap_public_header(
        self,
        poq_verification_input: PublicInputs,
    ) -> Result<UnwrappedEncapsulatedMessage<ENCAPSULATION_COUNT>, Error> {
        self.public_header.verify_signature(&signing_body(
            &self.encapsulated_part.private_header,
            &self.encapsulated_part.payload,
        ))?;
        let _ = self
            .public_header
            .verify_proof_of_quota(&poq_verification_input)?;
        Ok(UnwrappedEncapsulatedMessage::new(
            &poq_verification_input,
            *self.public_header.signing_pubkey(),
            self.encapsulated_part,
        ))
    }

    #[must_use]
    pub const fn id(&self) -> MessageIdentifier {
        *self.public_header.signing_pubkey()
    }

    #[cfg(feature = "unsafe-test-functions")]
    #[must_use]
    pub const fn public_header_mut(&mut self) -> &mut PublicHeader {
        &mut self.public_header
    }
}

/// Part of the message that should be encapsulated.
// TODO: Consider having `InitializedPart`
// that just finished the initialization step and doesn't have `decapsulate` method.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncapsulatedPart<const ENCAPSULATION_COUNT: usize> {
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
        proof_of_quota: &ProofOfQuota,
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
    pub(super) fn decapsulate(
        self,
        key: &SharedKey,
        poq_verification_input: &PublicInputs,
    ) -> Result<PartDecapsulationOutput<ENCAPSULATION_COUNT>, Error> {
        match self
            .private_header
            .decapsulate(key, poq_verification_input)?
        {
            PrivateHeaderDecapsulationOutput::Incompleted((private_header, public_header)) => {
                let payload = self.payload.decapsulate(key);
                verify_reconstructed_public_header(&public_header, &private_header, &payload)?;
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
                verify_reconstructed_public_header(&public_header, &private_header, &payload)?;
                Ok(PartDecapsulationOutput::Completed(
                    payload.try_deserialize()?,
                ))
            }
        }
    }

    /// Signs the encapsulated part using the provided key.
    fn sign(&self, key: &Ed25519PrivateKey) -> Signature {
        key.sign(&signing_body(&self.private_header, &self.payload))
    }
}

/// Verify the public header reconstructed when decapsulating the private
/// header.
fn verify_reconstructed_public_header<const ENCAPSULATION_COUNT: usize>(
    public_header: &PublicHeader,
    private_header: &EncapsulatedPrivateHeader<ENCAPSULATION_COUNT>,
    payload: &EncapsulatedPayload,
) -> Result<(), Error> {
    // Verify the signature in the reconstructed public header
    public_header.verify_signature(&signing_body(private_header, payload))
}

/// Returns the body that should be signed.
fn signing_body<const ENCAPSULATION_COUNT: usize>(
    private_header: &EncapsulatedPrivateHeader<ENCAPSULATION_COUNT>,
    payload: &EncapsulatedPayload,
) -> Vec<u8> {
    private_header
        .iter_bytes()
        .chain(payload.iter_bytes())
        .collect::<Vec<_>>()
}

/// An encapsulated private header, which is a set of encapsulated blending
/// headers.
// TODO: Consider having `InitializedPrivateHeader`
// that just finished the initialization step and doesn't have `decapsulate` method.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct EncapsulatedPrivateHeader<const ENCAPSULATION_COUNT: usize>(
    #[serde(with = "serde_big_array::BigArray")] [EncapsulatedBlendingHeader; ENCAPSULATION_COUNT],
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
                .map(|input| Some(input.ephemeral_encryption_key()))
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
                                .take_while_inclusive(|&input| {
                                    input.ephemeral_encryption_key() != rng_key
                                })
                                .for_each(|input| {
                                    header.encapsulate(input.ephemeral_encryption_key());
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
        proof_of_quota: &ProofOfQuota,
        signature: Signature,
        proof_of_selection: ProofOfSelection,
        is_last: bool,
    ) -> Self {
        // Shift blending headers by one rightward.
        self.shift_right();

        // Replace the first blending header with the new one.
        self.replace_first(EncapsulatedBlendingHeader::initialize(&BlendingHeader {
            signing_pubkey,
            proof_of_quota: *proof_of_quota,
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
        poq_verification_inputs: &PublicInputs,
    ) -> Result<PrivateHeaderDecapsulationOutput<ENCAPSULATION_COUNT>, Error> {
        // Decrypt all blending headers
        self.0.iter_mut().for_each(|header| {
            header.decapsulate(key);
        });

        // Check if the first blending header which was correctly decrypted
        // by verifying the decrypted proof of selection.
        // If the `private_key` is not correct, the proof of selection is
        // badly decrypted and verification will fail.
        let BlendingHeader {
            is_last,
            proof_of_quota,
            proof_of_selection,
            signature,
            signing_pubkey,
        } = self.first().try_deserialize()?;
        let _poq_nullifier = proof_of_quota
            .verify(*poq_verification_inputs)
            .map_err(|_| Error::ProofOfQuotaVerificationFailed)?;
        if !proof_of_selection.verify() {
            return Err(Error::ProofOfSelectionVerificationFailed);
        }

        // Build a new public header with the values in the first blending header.
        let public_header = PublicHeader::new(signing_pubkey, &proof_of_quota, signature);

        // Shift blending headers one leftward.
        self.shift_left();

        // Reconstruct/encrypt the last blending header
        // in the same way as the initialization step.
        let mut last_blending_header =
            EncapsulatedBlendingHeader::initialize(&BlendingHeader::pseudo_random(key.as_slice()));
        last_blending_header.encapsulate(key);
        self.replace_last(last_blending_header);

        if is_last {
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
