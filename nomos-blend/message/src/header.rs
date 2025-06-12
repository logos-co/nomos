use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

use crate::{
    crypto::{ProofOfQuota, ProofOfSelection, PublicKey, Signature},
    serde::FixedSizeSerde,
};

/// A header that contains basic metadata.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct Header {
    pub version: u8,
}

impl FixedSizeSerde for Header {
    const SIZE: usize = size_of::<u8>();
}

/// A public header that is not encapsulated.
/// This header is revealed to all nodes that receive the message.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct PublicHeader(pub BlendingSubHeader);

impl FixedSizeSerde for PublicHeader {
    const SIZE: usize = BlendingSubHeader::SIZE;
}

/// The maximum number of encapsulations allowed.
pub const MAX_ENCAPSULATIONS: usize = 3;

/// An private header that consists of multiple encapsulated blending headers.
/// Each blending header may be encapsulated in multiple layers,
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct PrivateHeader(pub [EncapsulatedBlendingHeader; MAX_ENCAPSULATIONS]);

impl FixedSizeSerde for PrivateHeader {
    const SIZE: usize = EncapsulatedBlendingHeader::SIZE * MAX_ENCAPSULATIONS;
}

/// A blending header that is encapsulated in at least one layer.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct EncapsulatedBlendingHeader(#[serde(with = "BigArray")] pub [u8; BlendingHeader::SIZE]);

impl FixedSizeSerde for EncapsulatedBlendingHeader {
    const SIZE: usize = size_of::<u8>() * BlendingHeader::SIZE;
}

/// A blending header that contains information for a next decapsulation.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct BlendingHeader {
    pub sub_header: BlendingSubHeader,
    /// A proof that the node corresponding to the [`public_key`] was selected
    /// for the current decapsulation.
    pub proof_of_selection: ProofOfSelection,
}

impl FixedSizeSerde for BlendingHeader {
    const SIZE: usize = BlendingSubHeader::SIZE + ProofOfSelection::SIZE;
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct BlendingSubHeader {
    /// A public key of Ephermeral Signing Key (ESK)
    pub public_key: PublicKey,
    /// A proof of quota corresponding to the [`public_key`]
    pub proof_of_quota: ProofOfQuota,
    /// A signature of the encapsulated private header and payload
    pub signature: Signature,
}

impl FixedSizeSerde for BlendingSubHeader {
    const SIZE: usize = PublicKey::SIZE + ProofOfQuota::SIZE + Signature::SIZE;
}
