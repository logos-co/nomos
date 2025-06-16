use bytes::BytesMut;

use crate::{
    crypto::{
        pseudo_random_bytes, ProofOfSelection, KEY_SIZE, PROOF_OF_QUOTA_SIZE,
        PROOF_OF_SELECTION_SIZE, SIGNATURE_SIZE,
    },
    message::{Message, MessageError, MAX_ENCAPSULATIONS, MAX_PAYLOAD_BODY_SIZE},
};

/// The size of a Blend message, including all headers and payload.
pub const MESSAGE_SIZE: usize = View::SIZE;

/// Projects the structure of a Blend message without allocating/copying memory.
pub struct View<'a> {
    pub header: Header<'a>,
    pub public_header: PublicHeader<'a>,
    pub private_header: PrivateHeader<'a>,
    pub payload: Payload<'a>,
}

impl<'a> View<'a> {
    const SIZE: usize = Header::SIZE + PublicHeader::SIZE + PrivateHeader::SIZE + Payload::SIZE;

    /// Parse a Blend message without allocating/copying memory,
    /// ensuring that all fields are completely filled with the expected sizes.
    pub fn parse(bytes: &'a mut Message) -> Result<Self, MessageError> {
        let (header, bytes) = Header::parse(&mut bytes.0)?;
        let (public_header, bytes) = PublicHeader::parse(bytes)?;
        let (private_header, bytes) = PrivateHeader::parse(bytes)?;
        let payload = Payload::parse(bytes)?;
        Ok(Self {
            header,
            public_header,
            private_header,
            payload,
        })
    }
}

pub struct Header<'a> {
    pub version: &'a mut u8,
}

impl<'a> Header<'a> {
    const SIZE: usize = size_of::<u8>();

    /// Parse a header without allocating/copying memory,
    /// ensuring that all fields are completely filled with the expected sizes.
    fn parse(bytes: &'a mut BytesMut) -> Result<(Self, &'a mut [u8]), MessageError> {
        let (version, bytes) = bytes
            .split_first_mut()
            .ok_or(MessageError::InvalidMessageFormat)?;
        Ok((Self { version }, bytes))
    }

    pub const fn fill(&mut self, version: u8) {
        *self.version = version;
    }
}

pub struct PublicHeader<'a> {
    pub signing_pubkey: &'a mut [u8],
    pub proof_of_quota: &'a mut [u8],
    pub signature: &'a mut [u8],
}

impl<'a> PublicHeader<'a> {
    const SIZE: usize = KEY_SIZE + PROOF_OF_QUOTA_SIZE + SIGNATURE_SIZE;

    /// Parse a public header without allocating/copying memory,
    /// ensuring that all fields are completely filled with the expected sizes.
    fn parse(bytes: &'a mut [u8]) -> Result<(Self, &'a mut [u8]), MessageError> {
        let (signing_pubkey, bytes) = split_at(bytes, KEY_SIZE)?;
        let (proof_of_quota, bytes) = split_at(bytes, PROOF_OF_QUOTA_SIZE)?;
        let (signature, bytes) = split_at(bytes, SIGNATURE_SIZE)?;
        Ok((
            Self {
                signing_pubkey,
                proof_of_quota,
                signature,
            },
            bytes,
        ))
    }

    pub const fn fill(&mut self, signing_pubkey: &[u8], proof_of_quota: &[u8], signature: &[u8]) {
        self.signing_pubkey.copy_from_slice(signing_pubkey);
        self.proof_of_quota.copy_from_slice(proof_of_quota);
        self.signature.copy_from_slice(signature);
    }
}

pub struct PrivateHeader<'a>([BlendingHeader<'a>; MAX_ENCAPSULATIONS]);

impl<'a> PrivateHeader<'a> {
    const SIZE: usize = BlendingHeader::SIZE * MAX_ENCAPSULATIONS;

    /// Parse a private header without allocating/copying memory,
    /// ensuring that all fields are completely filled with the expected sizes.
    fn parse(bytes: &'a mut [u8]) -> Result<(Self, &'a mut [u8]), MessageError> {
        let mut headers = Vec::with_capacity(MAX_ENCAPSULATIONS);
        let mut rest = bytes;

        for _ in 0..MAX_ENCAPSULATIONS {
            let (header, bytes) = BlendingHeader::parse(rest)?;
            headers.push(header);
            rest = bytes;
        }

        Ok((Self(headers.try_into().unwrap()), rest))
    }

    /// Returns the `index`-th blending header in the private header.
    pub fn get(&mut self, index: usize) -> Option<&mut BlendingHeader<'a>> {
        self.0.get_mut(index)
    }

    /// Returns the first blending header in the private header.
    pub const fn first(&mut self) -> &mut BlendingHeader<'a> {
        self.0.first_mut().expect("PrivateHeader is not empty")
    }

    /// Returns the last blending header in the private header.
    pub const fn last(&mut self) -> &mut BlendingHeader<'a> {
        self.0.last_mut().expect("PrivateHeader is not empty")
    }

    /// Shifts all blending headers in the private header in the specified
    /// direction with maintaining the size of the private header.
    pub fn shift(&mut self, direction: ShiftDirection) {
        let len = self.0.len();
        match direction {
            ShiftDirection::Right => {
                // h1 h2 h3 h4 -> h1 h1 h2 h3
                (1..len).rev().for_each(|i| {
                    let (left, right) = self.0.split_at_mut(i);
                    right[0].copy_from(&left[i - 1]);
                });
            }
            ShiftDirection::Left => {
                // h1 h2 h3 h4 -> h2 h3 h4 h4
                (1..len).for_each(|i| {
                    let (left, right) = self.0.split_at_mut(i);
                    left[i - 1].copy_from(&right[0]);
                });
            }
        }
    }
}

#[derive(Copy, Clone)]
pub enum ShiftDirection {
    Right,
    Left,
}

/// Represents a blending header that may be encrypted or unencrypted.
#[derive(Debug)]
pub struct BlendingHeader<'a> {
    pub signing_pubkey: &'a mut [u8],
    pub proof_of_quota: &'a mut [u8],
    pub signature: &'a mut [u8],
    pub proof_of_selection: &'a mut [u8],
}

impl<'a> BlendingHeader<'a> {
    const SIZE: usize = KEY_SIZE + PROOF_OF_QUOTA_SIZE + SIGNATURE_SIZE + PROOF_OF_SELECTION_SIZE;

    /// Parse a blending header without allocating/copying memory,
    /// ensuring that all fields are completely filled with the expected sizes.
    fn parse(bytes: &'a mut [u8]) -> Result<(Self, &'a mut [u8]), MessageError> {
        let (signing_pubkey, bytes) = split_at(bytes, KEY_SIZE)?;
        let (proof_of_quota, bytes) = split_at(bytes, PROOF_OF_QUOTA_SIZE)?;
        let (signature, bytes) = split_at(bytes, SIGNATURE_SIZE)?;
        let (proof_of_selection, bytes) = split_at(bytes, PROOF_OF_SELECTION_SIZE)?;
        Ok((
            Self {
                signing_pubkey,
                proof_of_quota,
                signature,
                proof_of_selection,
            },
            bytes,
        ))
    }

    pub fn proof_of_selection(&self) -> ProofOfSelection {
        ProofOfSelection(
            self.proof_of_selection
                .to_vec()
                .try_into()
                .expect("proof_of_selection must have exact amount of bytes"),
        )
    }

    /// Fills the blending header with random data based on the provided key
    /// in the way defined in the spec.
    /// Each field in the header is filled with pseudo-random bytes derived from
    /// the key concatenated with a unique byte (1, 2, 3, or 4).
    pub fn fill_random(&mut self, key: &[u8]) {
        self.fill(
            &pseudo_random_bytes(&concat(key, &[1]), self.signing_pubkey.len()),
            &pseudo_random_bytes(&concat(key, &[2]), self.proof_of_quota.len()),
            &pseudo_random_bytes(&concat(key, &[3]), self.signature.len()),
            &pseudo_random_bytes(&concat(key, &[4]), self.proof_of_selection.len()),
        );
    }

    pub const fn fill(
        &mut self,
        signing_pubkey: &[u8],
        proof_of_quota: &[u8],
        signature: &[u8],
        proof_of_selection: &[u8],
    ) {
        self.signing_pubkey.copy_from_slice(signing_pubkey);
        self.proof_of_quota.copy_from_slice(proof_of_quota);
        self.signature.copy_from_slice(signature);
        self.proof_of_selection.copy_from_slice(proof_of_selection);
    }

    /// Replaces this blending header with the values of the provided one.
    const fn copy_from(&mut self, header: &Self) {
        self.signing_pubkey.copy_from_slice(header.signing_pubkey);
        self.proof_of_quota.copy_from_slice(header.proof_of_quota);
        self.signature.copy_from_slice(header.signature);
        self.proof_of_selection
            .copy_from_slice(header.proof_of_selection);
    }
}

fn concat(a: &[u8], b: &[u8]) -> Vec<u8> {
    itertools::chain(a, b).copied().collect::<Vec<_>>()
}

/// Represents a payload containing a header and a padded body.
/// The length of the padded body is fixed to [`MAX_PAYLOAD_BODY_SIZE`].
pub struct Payload<'a> {
    pub header: PayloadHeader<'a>,
    pub body: &'a mut [u8],
}

impl<'a> Payload<'a> {
    const SIZE: usize = PayloadHeader::SIZE + MAX_PAYLOAD_BODY_SIZE;

    /// Parse a payload without allocating/copying memory,
    /// ensuring that all fields are completely filled with the expected sizes.
    fn parse(bytes: &'a mut [u8]) -> Result<Self, MessageError> {
        let (header, body) = PayloadHeader::parse(bytes)?;
        Ok(Self { header, body })
    }

    pub fn fill(&mut self, payload_type: u8, body: &[u8]) -> Result<(), MessageError> {
        if body.len() > MAX_PAYLOAD_BODY_SIZE {
            return Err(MessageError::PayloadTooLarge);
        }
        self.header.fill(
            payload_type,
            u16::try_from(body.len()).map_err(|_| MessageError::PayloadTooLarge)?,
        );
        self.body[..body.len()].copy_from_slice(body);
        Ok(())
    }

    /// Returns the payload body unpadded.
    #[cfg(test)] // TODO: remove this cfg once this function is used in upper layers
    pub fn body(&self) -> Result<&[u8], MessageError> {
        let len = u16::from_le_bytes(
            self.header
                .body_len
                .to_vec()
                .try_into()
                .map_err(|_| MessageError::InvalidPayloadLen)?,
        ) as usize;
        if self.body.len() < len {
            return Err(MessageError::InvalidPayloadLen);
        }
        Ok(&self.body[..len])
    }
}

pub struct PayloadHeader<'a> {
    pub payload_type: &'a mut u8,
    pub body_len: &'a mut [u8],
}

impl<'a> PayloadHeader<'a> {
    const SIZE: usize = size_of::<u8>() + size_of::<u16>();

    /// Parse a payload header without allocating/copying memory,
    /// ensuring that all fields are completely filled with the expected sizes.
    fn parse(bytes: &'a mut [u8]) -> Result<(Self, &'a mut [u8]), MessageError> {
        let (payload_type, bytes) = bytes
            .split_first_mut()
            .ok_or(MessageError::InvalidMessageFormat)?;
        let (body_len, bytes) = split_at(bytes, size_of::<u16>())?;
        Ok((
            Self {
                payload_type,
                body_len,
            },
            bytes,
        ))
    }

    pub const fn fill(&mut self, payload_type: u8, body_len: u16) {
        *self.payload_type = payload_type;
        self.body_len.copy_from_slice(&body_len.to_le_bytes());
    }
}

/// Projects a Blend message into the large semantic units
/// without allocating/copying memory
/// for efficient memory manipulations (signing and encryption).
pub struct RegionView<'a>(&'a mut Message);

impl<'a> RegionView<'a> {
    pub const fn new(message: &'a mut Message) -> Self {
        Self(message)
    }

    pub fn signing_body(&self) -> &[u8] {
        &self.0 .0[Header::SIZE + PublicHeader::SIZE..]
    }

    pub fn blending_header_mut(&mut self, index: usize) -> &mut [u8] {
        assert!(index < MAX_ENCAPSULATIONS);
        let pos = Header::SIZE + PublicHeader::SIZE + index * BlendingHeader::SIZE;
        &mut self.0 .0[pos..pos + BlendingHeader::SIZE]
    }

    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.0 .0[Header::SIZE + PublicHeader::SIZE + PrivateHeader::SIZE..]
    }
}

/// Splits a mutable byte slice into two parts at the specified size,
/// ensuring that the first part has the exact size and the second part
/// contains all the remaining bytes.
fn split_at(bytes: &mut [u8], size: usize) -> Result<(&mut [u8], &mut [u8]), MessageError> {
    bytes
        .split_at_mut_checked(size)
        .ok_or(MessageError::InvalidMessageFormat)
}
