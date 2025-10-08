use core::iter::repeat_n;

use itertools::Itertools as _;
use nomos_core::{
    codec::{DeserializeOp as _, SerializeOp as _},
    crypto::ZkHash,
};
use serde::{Deserialize, Serialize};

use crate::{
    Error, PayloadType,
    crypto::{
        keys::{Ed25519PrivateKey, Ed25519PublicKey, SharedKey},
        proofs::{
            quota::{self, ProofOfQuota, inputs::prove::PublicInputs},
            selection::{self, ProofOfSelection, inputs::VerifyInputs},
        },
        random_sized_bytes,
        signatures::Signature,
    },
    encap::{
        ProofsVerifier,
        decapsulated::{PartDecapsulationOutput, PrivateHeaderDecapsulationOutput},
        validated::IncomingEncapsulatedMessageWithValidatedPublicHeader,
    },
    input::EncapsulationInputs,
    message::{BlendingHeader, Payload, PublicHeader},
};

pub type MessageIdentifier = Ed25519PublicKey;

/// An encapsulated message that is sent to the blend network.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncapsulatedMessage<const ENCAPSULATION_COUNT: usize> {
    /// A public header that is not encapsulated.
    public_header: PublicHeader,
    /// Encapsulated parts
    encapsulated_part: EncapsulatedPart<ENCAPSULATION_COUNT>,
}

/// The inputs required to verify a Proof of Quota, without the signing key,
/// which is retrieved from the public header of the message layer being
/// veified.
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Default))]
pub struct PoQVerificationInputMinusSigningKey {
    pub session: u64,
    pub core_root: ZkHash,
    pub pol_ledger_aged: ZkHash,
    pub pol_epoch_nonce: ZkHash,
    pub core_quota: u64,
    pub leader_quota: u64,
    pub total_stake: u64,
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
                ProofOfQuota::from_bytes_unchecked(random_sized_bytes()),
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

    /// Consume the message to return its components.
    #[must_use]
    pub fn into_components(self) -> (PublicHeader, EncapsulatedPart<ENCAPSULATION_COUNT>) {
        (self.public_header, self.encapsulated_part)
    }

    /// Verify the message public header.
    pub fn verify_public_header<Verifier>(
        self,
        PoQVerificationInputMinusSigningKey {
            core_quota,
            core_root,
            leader_quota,
            pol_epoch_nonce,
            pol_ledger_aged,
            session,
            total_stake,
        }: &PoQVerificationInputMinusSigningKey,
        verifier: &Verifier,
    ) -> Result<IncomingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>, Error>
    where
        Verifier: ProofsVerifier,
    {
        // Verify signature according to the Blend spec: <https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81859cebf5e3d2a5cd8f>.
        self.public_header.verify_signature(&signing_body(
            &self.encapsulated_part.private_header,
            &self.encapsulated_part.payload,
        ))?;
        let (_, signing_key, proof_of_quota, signature) = self.public_header.into_components();
        // Verify the Proof of Quota according to the Blend spec: <https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81b593ddce00cffd24a8>.
        verifier
            .verify_proof_of_quota(
                proof_of_quota,
                &PublicInputs {
                    core_quota: *core_quota,
                    core_root: *core_root,
                    leader_quota: *leader_quota,
                    pol_epoch_nonce: *pol_epoch_nonce,
                    pol_ledger_aged: *pol_ledger_aged,
                    session: *session,
                    // Signing key taken from the public header after the signature has been
                    // successfully verified.
                    signing_key,
                    total_stake: *total_stake,
                },
            )
            .map_err(|_| Error::ProofOfQuotaVerificationFailed(quota::Error::InvalidProof))?;
        Ok(
            IncomingEncapsulatedMessageWithValidatedPublicHeader::from_message(
                Self::from_components(
                    PublicHeader::new(signing_key, &proof_of_quota, signature),
                    self.encapsulated_part,
                ),
            ),
        )
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
    pub(super) fn decapsulate<Verifier>(
        self,
        key: &SharedKey,
        posel_verification_input: &VerifyInputs,
        verifier: &Verifier,
    ) -> Result<PartDecapsulationOutput<ENCAPSULATION_COUNT>, Error>
    where
        Verifier: ProofsVerifier,
    {
        match self
            .private_header
            .decapsulate(key, posel_verification_input, verifier)?
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

    fn decapsulate<Verifier>(
        mut self,
        key: &SharedKey,
        posel_verification_input: &VerifyInputs,
        verifier: &Verifier,
    ) -> Result<PrivateHeaderDecapsulationOutput<ENCAPSULATION_COUNT>, Error>
    where
        Verifier: ProofsVerifier,
    {
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
        // Verify PoSel according to the Blend spec: <https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df81dd8cbedc8af4649a6a>.
        verifier
            .verify_proof_of_selection(proof_of_selection, posel_verification_input)
            .map_err(|_| {
                Error::ProofOfSelectionVerificationFailed(selection::Error::Verification)
            })?;

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
        Self(
            header
                .to_bytes()
                .expect("BlendingHeader should be able to be serialized")
                .to_vec(),
        )
    }

    /// Try to deserialize into a [`BlendingHeader`].
    /// If there is no encapsulation left, and if the bytes are valid,
    /// the deserialization will succeed.
    fn try_deserialize(&self) -> Result<BlendingHeader, Error> {
        BlendingHeader::from_bytes(&self.0).map_err(|_| Error::DeserializationFailed)
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
        Self(
            payload
                .to_bytes()
                .expect("Payload should be able to be serialized")
                .to_vec(),
        )
    }

    /// Try to deserialize into a [`Payload`].
    /// If there is no encapsulation left, and if the bytes are valid,
    /// the deserialization will succeed.
    fn try_deserialize(&self) -> Result<Payload, Error> {
        Payload::from_bytes(&self.0).map_err(|_| Error::DeserializationFailed)
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
