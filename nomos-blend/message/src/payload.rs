use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;
use serde_repr::{Deserialize_repr, Serialize_repr};

use crate::serde::FixedSizeSerde;

/// A payload that is encapsulated in at least one layer.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct EncapsulatedPayload(#[serde(with = "BigArray")] pub [u8; Payload::SIZE]);

impl FixedSizeSerde for EncapsulatedPayload {
    const SIZE: usize = size_of::<u8>() * Payload::SIZE;
}

const MAX_PAYLOAD_BODY_LEN: u16 = 34000;

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct Payload {
    header: PayloadHeader,
    #[serde(with = "BigArray")]
    padded_body: [u8; MAX_PAYLOAD_BODY_LEN as usize],
}

impl FixedSizeSerde for Payload {
    const SIZE: usize = PayloadHeader::SIZE + size_of::<u8>() * MAX_PAYLOAD_BODY_LEN as usize;
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct PayloadHeader {
    body_type: PayloadBodyType,
    body_length: u16,
}

impl FixedSizeSerde for PayloadHeader {
    const SIZE: usize = PayloadBodyType::SIZE + size_of::<u16>();
}

#[repr(u8)]
#[derive(Serialize_repr, Deserialize_repr, PartialEq, Eq, Debug, Copy, Clone)]
pub enum PayloadBodyType {
    COVER = 0x00,
    DATA = 0x01,
}

impl FixedSizeSerde for PayloadBodyType {
    const SIZE: usize = size_of::<u8>();
}
