use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::Error;

pub const MAX_PAYLOAD_BODY_SIZE: usize = 34 * 1024;

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

/// A payload that is fully decapsulated.
/// This must be encapsulated when being sent to the blend network.
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct Payload {
    header: PayloadHeader,
    /// A body is padded to [`MAX_PAYLOAD_BODY_SIZE`],
    /// Box is used to not allocate a big array on the stack.
    #[serde_as(as = "serde_with::Bytes")]
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

    pub fn try_into_components(self) -> Result<(PayloadType, Vec<u8>), Error> {
        Ok((self.payload_type(), self.body()?.to_vec()))
    }
}
