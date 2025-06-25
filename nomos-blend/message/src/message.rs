use serde::{Deserialize, Serialize};
use serde_with::{serde_as, Bytes};

use crate::{
    crypto::{
        pseudo_random_sized_bytes, random_sized_bytes, Ed25519PrivateKey, Ed25519PublicKey,
        ProofOfQuota, ProofOfSelection, Signature, KEY_SIZE, PROOF_OF_QUOTA_SIZE,
        PROOF_OF_SELECTION_SIZE, SIGNATURE_SIZE,
    },
    error::Error,
};

pub const VERSION: u8 = 1;

// A message header that is revealed to all nodes.
#[derive(Clone, Serialize, Deserialize)]
pub struct Header {
    version: u8,
}

impl Header {
    /// Create a header with the current version
    /// since this module implements the blend protocol version 1.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self { version: VERSION }
    }
}

impl Default for Header {
    fn default() -> Self {
        Self::new()
    }
}

// A public header that is revealed to all nodes.
#[derive(Clone, Serialize, Deserialize)]
pub struct PublicHeader {
    pub signing_pubkey: Ed25519PublicKey,
    pub proof_of_quota: ProofOfQuota,
    pub signature: Signature,
}

impl PublicHeader {
    pub fn verify_signature(&self, body: &[u8]) -> Result<(), Error> {
        if self.signing_pubkey.verify_signature(body, &self.signature) {
            Ok(())
        } else {
            Err(Error::SignatureVerificationFailed)
        }
    }
}

/// A blending header that is fully decapsulated.
/// This must be encapsulated when being sent to the blend network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlendingHeader {
    pub signing_pubkey: Ed25519PublicKey,
    pub proof_of_quota: ProofOfQuota,
    pub signature: Signature,
    pub proof_of_selection: ProofOfSelection,
    pub is_last: bool,
}

impl BlendingHeader {
    /// Build a blending header with random data based on the provided key.
    /// in the reconstructable way.
    /// Each field in the header is filled with pseudo-random bytes derived from
    /// the key concatenated with a unique byte (1, 2, 3, or 4).
    pub fn pseudo_random(key: &[u8]) -> Self {
        let r1 = pseudo_random_sized_bytes::<KEY_SIZE>(&concat(key, &[1]));
        let r2 = pseudo_random_sized_bytes::<PROOF_OF_QUOTA_SIZE>(&concat(key, &[2]));
        let r3 = pseudo_random_sized_bytes::<SIGNATURE_SIZE>(&concat(key, &[3]));
        let r4 = pseudo_random_sized_bytes::<PROOF_OF_SELECTION_SIZE>(&concat(key, &[4]));
        Self {
            // Unlike the spec, derive a private key from random bytes
            // and then derive the public key from it
            // because a public key cannot always be successfully derived from random bytes.
            // TODO: This will be changed once we have zerocopy serde.
            signing_pubkey: Ed25519PrivateKey::from(r1).public_key(),
            proof_of_quota: ProofOfQuota::from(r2),
            signature: Signature::from(r3),
            proof_of_selection: ProofOfSelection::from(r4),
            is_last: false,
        }
    }

    /// Build a blending header with random data that is not reconstructable.
    pub fn random() -> Self {
        // A random key is used since no reconstruction is needed.
        Self::pseudo_random(&random_sized_bytes::<KEY_SIZE>())
    }
}

fn concat(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.iter().chain(b.iter()).copied().collect::<Vec<_>>()
}

pub const MAX_PAYLOAD_BODY_SIZE: usize = 34 * 1024;

/// A payload that is fully decapsulated.
/// This must be encapsulated when being sent to the blend network.
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct Payload {
    header: PayloadHeader,
    /// A body is padded to [`MAX_PAYLOAD_BODY_SIZE`],
    /// Box is used to not allocate a big array on the stack.
    #[serde_as(as = "Bytes")]
    body: Box<[u8; MAX_PAYLOAD_BODY_SIZE]>,
}

impl Payload {
    pub fn new(payload_type: PayloadType, body: &[u8]) -> Result<Self, Error> {
        if body.len() > MAX_PAYLOAD_BODY_SIZE {
            return Err(Error::PayloadTooLarge);
        }
        let body_len: u16 = body
            .len()
            .try_into()
            .map_err(|_| Error::InvalidPayloadLength)?;
        let mut padded_body: Box<[u8; MAX_PAYLOAD_BODY_SIZE]> = vec![0; MAX_PAYLOAD_BODY_SIZE]
            .into_boxed_slice()
            .try_into()
            .expect("body must be created with the correct size");
        padded_body[..body.len()].copy_from_slice(body);

        Ok(Self {
            header: PayloadHeader {
                payload_type,
                body_len,
            },
            body: padded_body,
        })
    }

    pub const fn payload_type(&self) -> PayloadType {
        self.header.payload_type
    }

    /// Returns the payload body unpadded.
    /// Returns an error if the payload cannot be read up to the length
    /// specified in the header
    pub fn body(&self) -> Result<&[u8], Error> {
        let len = self.header.body_len as usize;
        if self.body.len() < len {
            return Err(Error::InvalidPayloadLength);
        }
        Ok(&self.body[..len])
    }
}

/// A payload header that is fully decapsulated.
/// This must be encapsulated when being sent to the blend network.
#[derive(Clone, Serialize, Deserialize)]
struct PayloadHeader {
    payload_type: PayloadType,
    body_len: u16,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PayloadType {
    Cover = 0x00,
    Data = 0x01,
}
