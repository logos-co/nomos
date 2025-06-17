use std::env::consts::OS;

use bytes::BytesMut;
use rand_chacha::rand_core::{OsRng, RngCore};
use rkyv::{munge::munge, rancor::Error, Archive, Deserialize, Serialize};

use crate::crypto::{
    pseudo_random_bytes, Ed25519PrivateKey, Ed25519PublicKey, ProofOfQuota, ProofOfSelection,
    SharedKey, Signature, X25519PrivateKey, KEY_SIZE, PROOF_OF_QUOTA_SIZE, PROOF_OF_SELECTION_SIZE,
    SIGNATURE_SIZE,
};

pub const VERSION: u8 = 1;
// TODO: Make these generic const parameters.
pub const MAX_ENCAPSULATIONS: usize = 3;
pub const MAX_PAYLOAD_BODY_SIZE: usize = 34 * 1024;

/// Represents a fixed-size Blend message.
///
/// This message is serialized using the [`rkyv`] crate, which uses:
/// - No length prefix and no alignment padding.
/// - Litten-endian byte order.
///
/// This message is deserialized using the [`rkyv`] crate, which implememts
/// zero-copy deserialization, for the following reasons:
/// - Message size is not small but fixed and known at compile time.
/// - Encapsulation/Decapsulation involves multiple memory manipulations, such
///   as encryption and shifting.
/// - Encryption/decryption should (can) be efficiently performed on the
///   multiple fields contiguous in memory.
/// - Only one node in the network can decapsulate the message. All other nodes
///   don't need to deserialize all fields that they will not be able to
///   decrypt.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
struct Message {
    header: Header,
    public_header: PublicHeader,
    private_header: PrivateHeader,
    payload: Payload,
}

impl Message {
    const SIZE: usize = Header::SIZE + PublicHeader::SIZE + PrivateHeader::SIZE + Payload::SIZE;

    pub fn encapsulate(
        inputs: &[EncapsulationInput],
        payload: &[u8],
    ) -> Result<Vec<u8>, MessageError> {
        if inputs.len() > MAX_ENCAPSULATIONS {
            return Err(MessageError::MaxEncapsulationsExceeded);
        }
        if payload.len() > MAX_PAYLOAD_BODY_SIZE {
            return Err(MessageError::PayloadTooLarge);
        }

        let buf = Self::initialize(inputs, payload)?;
        Self::encapsulate_all(buf, inputs)
    }

    fn initialize(
        inputs: &[EncapsulationInput],
        payload_body: &[u8],
    ) -> Result<Vec<u8>, MessageError> {
        let mut buf = vec![0u8; Self::SIZE];
        let msg = rkyv::access_mut::<ArchivedMessage, Error>(&mut buf)?;
        munge!(
            let ArchivedMessage {
                header,
                private_header,
                payload,
                ..
            } = msg;
            let ArchivedHeader {
                mut version,
            } = header;
            let ArchivedPrivateHeader(
                blending_headers,
            ) = private_header;
            let ArchivedPayload {
                header,
                mut body,
            } = payload;
            let ArchivedPayloadHeader {
                mut payload_type,
                mut body_len,
            } = header;
        );

        // Fill the header
        *version = VERSION;

        // Randomize blending headers that don't need to be filled with reconstructed data.
        blending_headers
            .iter()
            .take(blending_headers.len() - inputs.len())
            .for_each(|header| {
                let ArchivedBlendingHeader {
                    mut signing_pubkey,
                    mut proof_of_quota,
                    mut signature,
                    mut proof_of_selection,
                } = header;
                OsRng.fill_bytes(&mut signing_pubkey);
                OsRng.fill_bytes(&mut proof_of_quota);
                OsRng.fill_bytes(&mut signature);
                OsRng.fill_bytes(&mut proof_of_selection);
            });

        // Fill the last `inputs.len()` blending headers with reconstructed data.
        blending_headers
            .iter()
            .skip(blending_headers.len() - inputs.len())
            .zip(inputs.iter().rev())
            .for_each(|(header, input)| {
                let ArchivedBlendingHeader {
                    mut signing_pubkey,
                    mut proof_of_quota,
                    mut signature,
                    mut proof_of_selection,
                } = header;
                let key = input.shared_key.as_slice();
                let r1 = pseudo_random_bytes(&concat(key, &[1]), header.signing_pubkey.len());
                let r2 = pseudo_random_bytes(&concat(key, &[2]), header.proof_of_quota.len());
                let r3 = pseudo_random_bytes(&concat(key, &[3]), header.signature.len());
                let r4 = pseudo_random_bytes(&concat(key, &[4]), header.proof_of_selection.len());
                signing_pubkey.copy_from_slice(&r1);
                proof_of_quota.copy_from_slice(&r2);
                signature.copy_from_slice(&r3);
                proof_of_selection.copy_from_slice(&r4);
            });

        // Fill the payload.
        if payload_body.len() > MAX_PAYLOAD_BODY_SIZE {
            return Err(MessageError::PayloadTooLarge);
        }
        *payload_type = 0x01; // TODO: for now, just a data message.
        *body_len = payload_body
            .len()
            .try_into()
            .map_err(|_| MessageError::InvalidPayloadLen)?;
        body[..payload_body.len()].copy_from_slice(payload_body);

        // Encrypt the last `shared_key.len()` blending headers.
        //
        //          ...
        //          BlendingHeaders[k-3]
        // E2(E1(E0(BlendingHeaders[k-2])))
        //    E1(E0(BlendingHeaders[k-1]))
        //       E0(BlendingHeaders[k])
        let msg = rkyv::access_mut::<ArchivedMessageEncryptionView, Error>(&mut buf)?;
        munge!(
            let ArchivedMessageEncryptionView {
                mut private_header,
                ..
            } = msg;
        );
        let total_blending_headers = private_header.len();
        let num_encapsulations = inputs.len();
        inputs.iter().enumerate().for_each(|(i, input)| {
            for j in 0..num_encapsulations - i {
                input.shared_key.encrypt(
                    private_header
                        .get_mut(j + total_blending_headers - num_encapsulations)
                        .expect("BlendingHeader should exist")
                        .as_mut_slice(),
                );
            }
            // private_header
            //     .iter_mut()
            //     .skip(total_blending_headers - inputs.len())
            //     .take(inputs.len() - i)
            //     .for_each(|blending_header| {
            //         input.shared_key.encrypt(blending_header.as_mut_slice());
            //     });
        });

        Ok(buf)
    }

    /// Encapsulates the message with all the provided inputs.
    fn encapsulate_all(
        mut buf: Vec<u8>,
        inputs: &[EncapsulationInput],
    ) -> Result<Vec<u8>, MessageError> {
        // Encapsulate message using each shared key.
        // let (mut buf, _) = inputs
        //     .iter()
        //     .try_fold((buf, None), |(buf, prev_input), input| {
        //         Self::encapsulate_once(buf, input, prev_input).map(|buf| (buf, Some(input)))
        //     })?;
        for (i, input) in inputs.iter().enumerate() {
            let prev_input = if i == 0 { None } else { Some(&inputs[i - 1]) };
            buf = Self::encapsulate_once(buf, input, prev_input)?;
        }

        // Construct the public header.
        let EncapsulationInput {
            signing_key: last_signing_key,
            proof_of_quota: last_proof_of_quota,
            ..
        } = inputs
            .last()
            .ok_or(MessageError::InvalidEncapsulationInputs)?;
        let msg = rkyv::access_mut::<ArchivedMessageSigningView, Error>(&mut buf)?;
        munge!(let ArchivedMessageSigningView {
            mut signing_body,
            ..
        } = msg);
        let sig = last_signing_key.sign(signing_body.as_mut_slice());
        let msg = rkyv::access_mut::<ArchivedMessage, Error>(&mut buf)?;
        munge!(
            let ArchivedMessage{
                public_header,
                ..
            } = msg;
            let ArchivedPublicHeader {
                mut signing_pubkey,
                mut proof_of_quota,
                mut signature,
            } = public_header;
        );
        signing_pubkey.copy_from_slice(last_signing_key.public_key().as_bytes());
        proof_of_quota.copy_from_slice(last_proof_of_quota.as_bytes());
        signature.copy_from_slice(sig.as_bytes());

        Ok(buf)
    }

    /// Encapsulates the message once with a given input.
    ///
    /// A `prev_input` should be provided, if exists, to construct a blending
    /// header. If not, a random signing key and proof will be used.
    fn encapsulate_once(
        mut buf: Vec<u8>,
        input: &EncapsulationInput,
        prev_input: Option<&EncapsulationInput>,
    ) -> Result<Vec<u8>, MessageError> {
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

        let msg = rkyv::access_mut::<ArchivedMessageSigningView, Error>(&mut buf)?;
        munge!(let ArchivedMessageSigningView {
            mut signing_body,
            ..
        } = msg);
        let sig = signing_key.sign(signing_body.as_mut_slice());

        // Encrypt the payload using `input`.
        let msg = rkyv::access_mut::<ArchivedMessageEncryptionView, Error>(&mut buf)?;
        munge!(
            let ArchivedMessageEncryptionView {
                mut private_header,
                mut payload,
                ..
            } = msg;
        );
        input.shared_key.encrypt(payload.as_mut_slice());

        // Shift blending headers by one rightward.
        private_header.rotate_right(1);

        // Fill the first blending header.
        // The key, proof of quota, and signature of the `prev_input` should be used.
        // The proof of selection must be calculated using `input`.
        let msg = rkyv::access_mut::<ArchivedMessage, Error>(&mut buf)?;
        munge!(
            let ArchivedMessage {
                private_header,
                ..
            } = msg;
        );
        let ArchivedBlendingHeader {
            signing_pubkey: mut out_signing_pubkey,
            proof_of_quota: mut out_proof_of_quota,
            signature: mut out_signature,
            proof_of_selection: mut out_proof_of_selection,
        } = private_header
            .0
            .first()
            .ok_or(MessageError::InvalidMessageFormat)?;
        out_signing_pubkey.copy_from_slice(signing_pubkey.as_bytes());
        out_proof_of_quota.copy_from_slice(proof_of_quota.as_bytes());
        out_signature.copy_from_slice(sig.as_bytes());
        out_proof_of_selection.copy_from_slice(input.proof_of_selection.as_bytes());

        // Encrypt the private header using `input`.
        let msg = rkyv::access_mut::<ArchivedMessageEncryptionView, Error>(&mut buf)?;
        munge!(
            let ArchivedMessageEncryptionView {
                mut private_header,
                ..
            } = msg;
        );
        private_header.iter_mut().for_each(|blending_header| {
            input.shared_key.encrypt(blending_header.as_mut_slice());
        });

        Ok(buf)
    }

    /// Decapsulates the message using the provided encryption private key.
    ///
    /// If the `private_key` is not the one for decapsulating the first
    /// encapsulation, [`MessageError::ProofOfSelectionVerificationFailed`]
    /// will be returned. If it is the correct key, the first encapsulation
    /// will be decapsulated.
    pub fn decapsulate(
        mut buf: Vec<u8>,
        private_key: &X25519PrivateKey,
        // TODO: This should be replaced with a proper impl.
        verify_proof_of_selection: impl Fn(&ProofOfSelection) -> bool,
    ) -> Result<Vec<u8>, MessageError> {
        // Derive the shared key.
        let msg = rkyv::access::<ArchivedMessage, Error>(&buf)?;
        let shared_key = private_key.derive_shared_key(
            &Ed25519PublicKey::from_slice(msg.public_header.signing_pubkey.as_slice())
                .map_err(|_| MessageError::InvalidPublicKey)?
                .derive_x25519(),
        );

        // Decrypt the private header.
        let msg = rkyv::access_mut::<ArchivedMessageEncryptionView, Error>(&mut buf)?;
        munge!(let ArchivedMessageEncryptionView {
            mut private_header,
            ..
        } = msg);
        private_header.iter_mut().for_each(|blending_header| {
            shared_key.decrypt(blending_header.as_mut_slice());
        });

        // Check if the first blending header which was correctly decrypted
        // by verifying the decrypted proof of selection.
        // If the `private_key` is not correct, the proof of selection is
        // badly decrypted and verification will fail.
        let msg = rkyv::access::<ArchivedMessage, Error>(&buf)?;
        let proof_of_selection = ProofOfSelection(
            msg.private_header
                .0
                .first()
                .ok_or(MessageError::InvalidMessageFormat)?
                .proof_of_selection,
        );
        if !verify_proof_of_selection(&proof_of_selection) {
            return Err(MessageError::ProofOfSelectionVerificationFailed);
        }

        // Replace the public header with the values in the first blending header.
        let msg = rkyv::access_mut::<ArchivedMessage, Error>(&mut buf)?;
        munge!(
            let ArchivedMessage {
                public_header,
                private_header,
                ..
            } = msg;
            let ArchivedPublicHeader {
                mut signing_pubkey,
                mut proof_of_quota,
                mut signature,
            } = public_header;
        );
        let first_blending_header = private_header
            .0
            .first()
            .ok_or(MessageError::InvalidMessageFormat)?;
        signing_pubkey.copy_from_slice(first_blending_header.signing_pubkey.as_slice());
        proof_of_quota.copy_from_slice(first_blending_header.proof_of_quota.as_slice());
        signature.copy_from_slice(first_blending_header.signature.as_slice());

        // Decrypt the payload.
        let msg = rkyv::access_mut::<ArchivedMessageEncryptionView, Error>(&mut buf)?;
        munge!(
            let ArchivedMessageEncryptionView {
                mut private_header,
                mut payload,
                ..
            } = msg
        );
        shared_key.decrypt(payload.as_mut_slice());

        // Shift blending headers one leftward.
        private_header.rotate_left(1);

        // Reconstruct the last blending header
        let msg = rkyv::access_mut::<ArchivedMessage, Error>(&mut buf)?;
        munge!(
            let ArchivedMessage {
                private_header,
                ..
            } = msg
        );
        let ArchivedBlendingHeader {
            mut signing_pubkey,
            mut proof_of_quota,
            mut signature,
            mut proof_of_selection,
        } = private_header
            .0
            .last()
            .ok_or(MessageError::InvalidMessageFormat)?;
        let key = shared_key.as_slice();
        let r1 = pseudo_random_bytes(&concat(key, &[1]), signing_pubkey.len());
        let r2 = pseudo_random_bytes(&concat(key, &[2]), proof_of_quota.len());
        let r3 = pseudo_random_bytes(&concat(key, &[3]), signature.len());
        let r4 = pseudo_random_bytes(&concat(key, &[4]), proof_of_selection.len());
        signing_pubkey.copy_from_slice(&r1);
        proof_of_quota.copy_from_slice(&r2);
        signature.copy_from_slice(&r3);
        proof_of_selection.copy_from_slice(&r4);

        // Encrypt the reconstructed last blending header.
        let msg = rkyv::access_mut::<ArchivedMessageEncryptionView, Error>(&mut buf)?;
        munge!(let ArchivedMessageEncryptionView {
            mut private_header,
            ..
        } = msg);
        shared_key.encrypt(
            private_header
                .last_mut()
                .ok_or(MessageError::InvalidMessageFormat)?
                .as_mut_slice(),
        );

        // TODO: Compare signatures (The spec should be clarified).

        Ok(buf)
    }
}

/// A message header that contains the information about the message format.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    version: u8,
}

impl Header {
    const SIZE: usize = size_of::<u8>();
}

/// A public header that is revealed to all nodes in the network.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
struct PublicHeader {
    signing_pubkey: [u8; KEY_SIZE],
    proof_of_quota: [u8; PROOF_OF_QUOTA_SIZE],
    signature: [u8; SIGNATURE_SIZE],
}

impl PublicHeader {
    const SIZE: usize = KEY_SIZE + PROOF_OF_QUOTA_SIZE + SIGNATURE_SIZE;
}

/// A private header that can be decrypted by the selected blend nodes.
/// It consists of at most [`MAX_ENCAPSULATIONS`] blending headers,
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
struct PrivateHeader([BlendingHeader; MAX_ENCAPSULATIONS]);

impl PrivateHeader {
    const SIZE: usize = BlendingHeader::SIZE * MAX_ENCAPSULATIONS;
}

/// A blending header that can be revealed to only the selected blend node.
///
/// This contains the information that allows a node to determine whether
/// it can decapsulate it, as well as the data required for the decapsulation
/// process.
///
/// This struct may contain either plain data or data that has been encrypted
/// in multiple layers. Since length-preserving encryption is used, we use
/// the same struct regardless.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
struct BlendingHeader {
    signing_pubkey: [u8; KEY_SIZE],
    proof_of_quota: [u8; PROOF_OF_QUOTA_SIZE],
    signature: [u8; SIGNATURE_SIZE],
    proof_of_selection: [u8; PROOF_OF_SELECTION_SIZE],
}

impl BlendingHeader {
    const SIZE: usize = KEY_SIZE + PROOF_OF_QUOTA_SIZE + SIGNATURE_SIZE + PROOF_OF_SELECTION_SIZE;
}

/// A payload that contains a header and a body.
/// This can be revealed to the final blend node selected.
///
/// This struct may contain either plain data or data that has been encrypted
/// in multiple layers. Since length-preserving encryption is used, we use
/// the same struct regardless.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
struct Payload {
    header: PayloadHeader,
    body: [u8; MAX_PAYLOAD_BODY_SIZE],
}

impl Payload {
    const SIZE: usize = PayloadHeader::SIZE + MAX_PAYLOAD_BODY_SIZE;
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
struct PayloadHeader {
    payload_type: u8,
    body_len: u16,
}

impl PayloadHeader {
    const SIZE: usize = size_of::<u8>() + size_of::<u16>();
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
struct MessageSigningView {
    header_and_public_header: [u8; Header::SIZE + PublicHeader::SIZE],
    signing_body: [u8; PrivateHeader::SIZE + Payload::SIZE],
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
struct MessageEncryptionView {
    header_and_public_header: [u8; Header::SIZE + PublicHeader::SIZE],
    private_header: [[u8; BlendingHeader::SIZE]; MAX_ENCAPSULATIONS],
    payload: [u8; Payload::SIZE],
}

// impl Message {
//     /// Creates a new [`Message`] from a [`BytesMut`] instance.
//     /// The input must be exactly [`MESSAGE_SIZE`] bytes long.
//     fn new(bytes: BytesMut) -> Result<Self, MessageError> {
//         if bytes.len() != MESSAGE_SIZE {
//             return Err(MessageError::InvalidMessageSize);
//         }
//         Ok(Self(bytes))
//     }

//     /// Encapsulates the given payload with the provided encapsulation inputs.
//     /// Encryption inputs more than [`MAX_ENCAPSULATIONS`] are not allowed.
//     pub fn encapsulate(
//         inputs: &[EncapsulationInput],
//         payload: &[u8],
//     ) -> Result<Self, MessageError> {
//         if inputs.len() > MAX_ENCAPSULATIONS {
//             return Err(MessageError::MaxEncapsulationsExceeded);
//         }
//         Self::initialize(inputs, payload)?.encapsulate_all(inputs)
//     }

//     /// An initialization step defined in the spec to prepare encapsulations.
//     fn initialize(
//         inputs: &[EncapsulationInput],
//         payload_body: &[u8],
//     ) -> Result<Self, MessageError> {
//         assert!(inputs.len() <= MAX_ENCAPSULATIONS);

//         let mut msg = Self::new(BytesMut::zeroed(MESSAGE_SIZE))?;

//         // Fill the header.
//         let mut view = View::parse(&mut msg)?;
//         view.header.fill(VERSION);

//         // Randomize the private header.
//         for i in 0..MAX_ENCAPSULATIONS {
//             view.private_header
//                 .get(i)
//                 .expect("BlendingHeader should exist")
//                 .fill_random(Ed25519PrivateKey::generate().public_key().as_bytes());
//         }

//         // Fill the last `inputs.len()` blending headers with resonstructed data.
//         let num_encapsulations = inputs.len();
//         for (i, input) in inputs.iter().rev().enumerate() {
//             let blending_header = view
//                 .private_header
//                 .get(i + MAX_ENCAPSULATIONS - num_encapsulations)
//                 .expect("BlendingHeader should exist");
//             blending_header.fill_random(input.shared_key.as_slice());
//         }

//         // Fill the payload.
//         view.payload.fill(0x01, payload_body)?;

//         // Encrypt the last `shared_key.len()` blending headers.
//         //
//         //          ...
//         //          BlendingHeaders[k-3]
//         // E2(E1(E0(BlendingHeaders[k-2])))
//         //    E1(E0(BlendingHeaders[k-1]))
//         //       E0(BlendingHeaders[k])
//         let mut view = RegionView::new(&mut msg);
//         let num_encapsulations = inputs.len();
//         for (i, input) in inputs.iter().enumerate() {
//             for j in 0..num_encapsulations - i {
//                 input.shared_key.encrypt(
//                     view.blending_header_mut(j + MAX_ENCAPSULATIONS - num_encapsulations)?,
//                 );
//             }
//         }

//         Ok(msg)
//     }

//     /// Encapsulates the message with all the provided inputs.
//     fn encapsulate_all(self, inputs: &[EncapsulationInput]) -> Result<Self, MessageError> {
//         assert!(inputs.len() <= MAX_ENCAPSULATIONS);

//         let mut msg = self;

//         // Encapsulate message using each shared key.
//         for (i, input) in inputs.iter().enumerate() {
//             let prev_input = if i == 0 { None } else { Some(&inputs[i - 1]) };
//             msg = msg.encapsulate_once(input, prev_input)?;
//         }

//         // Construct the public header.
//         let EncapsulationInput {
//             signing_key,
//             proof_of_quota,
//             ..
//         } = inputs.last().unwrap();
//         let sig = signing_key.sign(RegionView::new(&mut msg).signing_body()?);
//         View::parse(&mut msg)?.public_header.fill(
//             signing_key.public_key().as_bytes(),
//             proof_of_quota.as_bytes(),
//             &sig,
//         );

//         Ok(msg)
//     }

//     /// Encapsulates the message once with a given input.
//     ///
//     /// A `prev_input` should be provided, if exists, to construct a blending
//     /// header. If not, a random signing key and proof will be used.
//     fn encapsulate_once(
//         self,
//         input: &EncapsulationInput,
//         prev_input: Option<&EncapsulationInput>,
//     ) -> Result<Self, MessageError> {
//         let mut msg = self;

//         // Generate a signature over the concatenation of the current private header and
//         // payload, using the signing key of the `prev_input`.
//         let (signing_key, signing_pubkey, proof_of_quota) = if let Some(EncapsulationInput {
//             signing_key,
//             proof_of_quota,
//             ..
//         }) = prev_input
//         {
//             let signing_pubkey = signing_key.public_key();
//             (signing_key.clone(), signing_pubkey, proof_of_quota.clone())
//         } else {
//             // If `prev_input` is `None`, use a random key and proof.
//             let signing_key = Ed25519PrivateKey::generate();
//             let signing_pubkey = signing_key.public_key();
//             let proof_of_quota = ProofOfQuota(
//                 pseudo_random_bytes(signing_key.as_bytes(), PROOF_OF_QUOTA_SIZE)
//                     .try_into()
//                     .unwrap(),
//             );
//             (signing_key, signing_pubkey, proof_of_quota)
//         };
//         let mut view = RegionView::new(&mut msg);
//         let sig = signing_key.sign(view.signing_body()?);

//         // Encrypt the payload using `input`.
//         input.shared_key.encrypt(view.payload_mut()?);

//         // Shift blending headers by one rightward.
//         let mut view = View::parse(&mut msg)?;
//         view.private_header.shift(ShiftDirection::Right);

//         // Fill the first blending header.
//         // The key, proof of quota, and signature of the `prev_input` should be used.
//         // The proof of selection must be calculated using `input`.
//         view.private_header.first().fill(
//             signing_pubkey.as_bytes(),
//             proof_of_quota.as_bytes(),
//             &sig,
//             input.proof_of_selection.as_bytes(),
//         );

//         // Encrypt the private header using `input`.
//         let mut view = RegionView::new(&mut msg);
//         for i in 0..MAX_ENCAPSULATIONS {
//             input.shared_key.encrypt(view.blending_header_mut(i)?);
//         }

//         Ok(msg)
//     }

//     /// Decapsulates the message using the provided encryption private key.
//     ///
//     /// If the `private_key` is not the one for decapsulating the first
//     /// encapsulation, [`MessageError::ProofOfSelectionVerificationFailed`]
//     /// will be returned. If it is the correct key, the first encapsulation
//     /// will be decapsulated.
//     pub fn decapsulate(
//         message: BytesMut,
//         private_key: &X25519PrivateKey,
//         // TODO: This should be replaced with a proper impl.
//         verify_proof_of_selection: impl Fn(&ProofOfSelection) -> bool,
//     ) -> Result<Self, MessageError> {
//         let mut msg = Self::new(message)?;

//         // Derive the shared key.
//         let view = View::parse(&mut msg)?;
//         let shared_key = private_key.derive_shared_key(
//             &Ed25519PublicKey::from_slice(view.public_header.signing_pubkey)
//                 .map_err(|_| MessageError::InvalidPublicKey)?
//                 .derive_x25519(),
//         );

//         // Decrypt the private header.
//         let mut view = RegionView::new(&mut msg);
//         for i in 0..MAX_ENCAPSULATIONS {
//             shared_key.decrypt(view.blending_header_mut(i)?);
//         }

//         // Check if the first blending header which was correctly decrypted
//         // by verifying the decrypted proof of selection.
//         // If the `private_key` is not correct, the proof of selection is
//         // badly decrypted and verification will fail.
//         let mut view = View::parse(&mut msg)?;
//         if !verify_proof_of_selection(&view.private_header.first().proof_of_selection()) {
//             return Err(MessageError::ProofOfSelectionVerificationFailed);
//         }

//         // Replace the public header with the values in the first blending header.
//         let first_blending_header = view.private_header.first();
//         view.public_header.fill(
//             first_blending_header.signing_pubkey,
//             first_blending_header.proof_of_quota,
//             first_blending_header.signature,
//         );

//         // Decrypt the payload.
//         let mut view = RegionView::new(&mut msg);
//         shared_key.decrypt(view.payload_mut()?);

//         // Shift blending headers one leftward.
//         let mut view = View::parse(&mut msg)?;
//         view.private_header.shift(ShiftDirection::Left);

//         // Reconstruct the last blending header
//         // in the same way as the initialization step.
//         view.private_header
//             .last()
//             .fill_random(shared_key.as_slice());

//         // Encrypt the reconstructed last blending header.
//         let mut view = RegionView::new(&mut msg);
//         shared_key.encrypt(view.blending_header_mut(MAX_ENCAPSULATIONS - 1)?);

//         // TODO: Compare signatures (The spec should be clarified).

//         Ok(msg)
//     }
// }

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

#[derive(thiserror::Error, Debug)]
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
    #[error("Invalid encapsulation inputs")]
    InvalidEncapsulationInputs,
    #[error("Serde error: {0}")]
    SerdeError(#[from] rkyv::rancor::Error),
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
        assert_eq!(msg.len(), Message::SIZE);

        // We expect that the decapsulations can be done
        // in the reverse order of blend_node_signing_keys.

        // We can decapsulate with the correct private key.
        let blend_node_signing_key = blend_node_signing_keys.last().unwrap();
        let msg = Message::decapsulate(
            msg,
            &blend_node_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == blend_node_signing_key.public_key().as_bytes()
            },
        )
        .unwrap();

        // We cannot decapsulate with an invalid private key,
        // which we already used for the first decapsulation.
        assert!(Message::decapsulate(
            msg.clone(),
            &blend_node_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == blend_node_signing_key.public_key().as_bytes()
            },
        )
        .is_err());

        // We can decapsulate with the correct private key
        let blend_node_signing_key = blend_node_signing_keys.first().unwrap();
        let mut msg = Message::decapsulate(
            msg,
            &blend_node_signing_key.derive_x25519(),
            |proof_of_selection| {
                proof_of_selection.as_bytes() == blend_node_signing_key.public_key().as_bytes()
            },
        )
        .unwrap();

        // The payload body should be the same as the original one.
        let msg = rkyv::access::<ArchivedMessage, Error>(&mut msg).unwrap();
        let body = msg
            .payload
            .body
            .split_at(msg.payload.header.body_len.into())
            .0;
        assert_eq!(body, PAYLOAD_BODY);
    }

    // #[test]
    // fn max_encapsulations_exceeded() {
    //     let (inputs, _) = generate_inputs(MAX_ENCAPSULATIONS + 1);
    //     assert_eq!(
    //         Message::encapsulate(&inputs, b"hello"),
    //         Err(MessageError::MaxEncapsulationsExceeded)
    //     );
    // }

    // #[test]
    // fn payload_too_long() {
    //     let (inputs, _) = generate_inputs(1);
    //     assert_eq!(
    //         Message::encapsulate(&inputs, &vec![0u8; MAX_PAYLOAD_BODY_SIZE + 1]),
    //         Err(MessageError::PayloadTooLarge)
    //     );
    // }

    // #[test]
    // fn decapsulate_invalid_sized_message() {
    //     assert_eq!(
    //         Message::decapsulate(
    //             BytesMut::zeroed(MESSAGE_SIZE + 1),
    //             &X25519PrivateKey::from_bytes([0u8; KEY_SIZE]),
    //             |_| true
    //         ),
    //         Err(MessageError::InvalidMessageSize)
    //     );
    // }

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

fn concat(a: &[u8], b: &[u8]) -> Vec<u8> {
    itertools::chain(a, b).copied().collect::<Vec<_>>()
}
