use bytes::BytesMut;

use crate::{
    crypto::{
        pseudo_random_bytes, ProofOfSelection, KEY_SIZE, PROOF_OF_QUOTA_SIZE,
        PROOF_OF_SELECTION_SIZE, SIGNATURE_SIZE,
    },
    message::{Message, MessageError, MAX_ENCAPSULATIONS, MAX_PAYLOAD_BODY_SIZE},
};

pub const MESSAGE_SIZE: usize = View::SIZE;

pub struct View<'a> {
    pub header: Header<'a>,
    pub public_header: PublicHeader<'a>,
    pub private_header: PrivateHeader<'a>,
    pub payload: Payload<'a>,
}

impl<'a> View<'a> {
    const SIZE: usize = Header::SIZE + PublicHeader::SIZE + PrivateHeader::SIZE + Payload::SIZE;

    pub fn new(bytes: &'a mut Message) -> Self {
        let (header, bytes) = Header::new(&mut bytes.0);
        let (public_header, bytes) = PublicHeader::new(bytes);
        let (private_header, bytes) = PrivateHeader::new(bytes);
        let payload = Payload::new(bytes);
        Self {
            header,
            public_header,
            private_header,
            payload,
        }
    }
}

pub struct Header<'a> {
    pub version: &'a mut u8,
}

impl<'a> Header<'a> {
    const SIZE: usize = size_of::<u8>();

    fn new(bytes: &'a mut BytesMut) -> (Self, &'a mut [u8]) {
        let (version, bytes) = bytes.split_first_mut().unwrap();
        (Self { version }, bytes)
    }
}

pub struct PublicHeader<'a> {
    pub signing_pubkey: &'a mut [u8],
    pub proof_of_quota: &'a mut [u8],
    pub signature: &'a mut [u8],
}

impl<'a> PublicHeader<'a> {
    const SIZE: usize = KEY_SIZE + PROOF_OF_QUOTA_SIZE + SIGNATURE_SIZE;

    const fn new(bytes: &'a mut [u8]) -> (Self, &'a mut [u8]) {
        let (signing_pubkey, bytes) = bytes.split_at_mut(KEY_SIZE);
        let (proof_of_quota, bytes) = bytes.split_at_mut(PROOF_OF_QUOTA_SIZE);
        let (signature, bytes) = bytes.split_at_mut(SIGNATURE_SIZE);
        (
            Self {
                signing_pubkey,
                proof_of_quota,
                signature,
            },
            bytes,
        )
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

    fn new(bytes: &'a mut [u8]) -> (Self, &'a mut [u8]) {
        let mut headers = Vec::with_capacity(MAX_ENCAPSULATIONS);
        let mut rest = bytes;

        for _ in 0..MAX_ENCAPSULATIONS {
            let (header, bytes) = BlendingHeader::new(rest);
            headers.push(header);
            rest = bytes;
        }

        (Self(headers.try_into().unwrap()), rest)
    }

    pub fn get(&mut self, index: usize) -> Option<&mut BlendingHeader<'a>> {
        self.0.get_mut(index)
    }

    pub const fn first(&mut self) -> &mut BlendingHeader<'a> {
        self.0.first_mut().expect("PrivateHeader is not empty")
    }

    pub const fn last(&mut self) -> &mut BlendingHeader<'a> {
        self.0.last_mut().expect("PrivateHeader is not empty")
    }

    pub fn shift(&mut self, direction: ShiftDirection) {
        let len = self.0.len();
        match direction {
            ShiftDirection::Right => {
                (1..len).rev().for_each(|i| {
                    let (left, right) = self.0.split_at_mut(i);
                    right[0].copy_from(&left[i - 1]);
                });
            }
            ShiftDirection::Left => {
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

#[derive(Debug)]
pub struct BlendingHeader<'a> {
    pub signing_pubkey: &'a mut [u8],
    pub proof_of_quota: &'a mut [u8],
    pub signature: &'a mut [u8],
    pub proof_of_selection: &'a mut [u8],
}

impl<'a> BlendingHeader<'a> {
    const SIZE: usize = KEY_SIZE + PROOF_OF_QUOTA_SIZE + SIGNATURE_SIZE + PROOF_OF_SELECTION_SIZE;

    const fn new(bytes: &'a mut [u8]) -> (Self, &'a mut [u8]) {
        let (signing_pubkey, bytes) = bytes.split_at_mut(KEY_SIZE);
        let (proof_of_quota, bytes) = bytes.split_at_mut(PROOF_OF_QUOTA_SIZE);
        let (signature, bytes) = bytes.split_at_mut(SIGNATURE_SIZE);
        let (proof_of_selection, bytes) = bytes.split_at_mut(PROOF_OF_SELECTION_SIZE);
        (
            Self {
                signing_pubkey,
                proof_of_quota,
                signature,
                proof_of_selection,
            },
            bytes,
        )
    }

    pub fn proof_of_selection(&self) -> ProofOfSelection {
        ProofOfSelection(
            self.proof_of_selection
                .to_vec()
                .try_into()
                .expect("proof_of_selection must have exact amount of bytes"),
        )
    }

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

pub struct Payload<'a> {
    pub header: PayloadHeader<'a>,
    pub body: &'a mut [u8],
}

impl<'a> Payload<'a> {
    const SIZE: usize = PayloadHeader::SIZE + MAX_PAYLOAD_BODY_SIZE;

    const fn new(bytes: &'a mut [u8]) -> Self {
        let (header, body) = PayloadHeader::new(bytes);
        Self { header, body }
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

    const fn new(bytes: &'a mut [u8]) -> (Self, &'a mut [u8]) {
        let (payload_type, bytes) = bytes.split_first_mut().unwrap();
        let (body_len, bytes) = bytes.split_at_mut(size_of::<u16>());
        (
            Self {
                payload_type,
                body_len,
            },
            bytes,
        )
    }

    pub const fn fill(&mut self, payload_type: u8, body_len: u16) {
        *self.payload_type = payload_type;
        self.body_len.copy_from_slice(&body_len.to_le_bytes());
    }
}

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
