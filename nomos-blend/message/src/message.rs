use serde::{Deserialize, Serialize};

use crate::{
    crypto::{Ed25519PublicKey, ProofOfQuota, ProofOfSelection, Signature},
    error::Error,
};

pub const VERSION: u8 = 1;

// A message header that is revealed to all nodes.
#[derive(Clone, Serialize, Deserialize)]
pub struct Header {
    pub version: u8,
}

impl Header {
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

/// A blending header that is fully decapsulated.
/// This must be encapsulated when being sent to the blend network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlendingHeader {
    pub signing_pubkey: Ed25519PublicKey,
    pub proof_of_quota: ProofOfQuota,
    pub signature: Signature,
    pub proof_of_selection: ProofOfSelection,
}

/// A payload that is fully decapsulated.
/// This must be encapsulated when being sent to the blend network.
#[derive(Clone, Serialize, Deserialize)]
pub struct Payload {
    pub header: PayloadHeader,
    /// A body is padded to [`MAX_PAYLOAD_BODY_SIZE`],
    /// Vec is used to not allocate a big array on the stack.
    pub body: Vec<u8>,
}

impl Payload {
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
pub struct PayloadHeader {
    pub payload_type: u8,
    pub body_len: u16,
}
