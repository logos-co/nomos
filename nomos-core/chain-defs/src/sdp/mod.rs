use std::hash::Hash;

use blake2::{Blake2b, Digest as _};
use multiaddr::Multiaddr;
use nom::{
    IResult, Parser as _,
    bytes::complete::take,
    number::complete::{le_u32, u8 as nom_u8},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::EnumIter;
use zksign::PublicKey;

use crate::{
    blend::{BlendingToken, PROOF_OF_QUOTA_SIZE, PROOF_OF_SELECTION_SIZE},
    block::BlockNumber,
    mantle::NoteId,
};

pub type SessionNumber = u64;
pub type StakeThreshold = u64;

const ACTIVE_METADATA_DA_TYPE: u8 = 0x00;
const ACTIVE_METADATA_BLEND_TYPE: u8 = 0x01;

const DA_ACTIVE_METADATA_VERSION_BYTE: u8 = 0x01;
type DaMetadataLengthPrefix = u32;
const DA_MIN_METADATA_SIZE: usize = 2 // type + version byte
+ size_of::<SessionNumber>() + size_of::<DaMetadataLengthPrefix>() * 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct MinStake {
    pub threshold: StakeThreshold,
    pub timestamp: BlockNumber,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceParameters {
    pub lock_period: u64,
    pub inactivity_period: u64,
    pub retention_period: u64,
    pub timestamp: BlockNumber,
    pub session_duration: BlockNumber,
}

impl ServiceParameters {
    #[must_use]
    pub const fn session_for_block(&self, block_number: BlockNumber) -> SessionNumber {
        block_number / self.session_duration
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Locator(pub Multiaddr);

impl Locator {
    #[must_use]
    pub const fn new(addr: Multiaddr) -> Self {
        Self(addr)
    }
}

impl AsRef<Multiaddr> for Locator {
    fn as_ref(&self) -> &Multiaddr {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, EnumIter)]
pub enum ServiceType {
    #[serde(rename = "BN")]
    BlendNetwork,
    #[serde(rename = "DA")]
    DataAvailability,
}

impl AsRef<str> for ServiceType {
    fn as_ref(&self) -> &str {
        match self {
            Self::BlendNetwork => "BN",
            Self::DataAvailability => "DA",
        }
    }
}

impl From<ServiceType> for usize {
    fn from(service_type: ServiceType) -> Self {
        match service_type {
            ServiceType::BlendNetwork => 0,
            ServiceType::DataAvailability => 1,
        }
    }
}

pub type Nonce = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ProviderId(pub ed25519_dalek::VerifyingKey);

#[derive(Debug)]
pub struct InvalidKeyBytesError;

impl TryFrom<[u8; 32]> for ProviderId {
    type Error = InvalidKeyBytesError;

    fn try_from(bytes: [u8; 32]) -> Result<Self, Self::Error> {
        ed25519_dalek::VerifyingKey::from_bytes(&bytes)
            .map(ProviderId)
            .map_err(|_| InvalidKeyBytesError)
    }
}

impl Serialize for ProviderId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            // For JSON: serialize as hex string
            const_hex::encode(self.0.as_bytes()).serialize(serializer)
        } else {
            // For binary: serialize as bytes
            self.0.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            // For JSON: deserialize from hex string
            let s = String::deserialize(deserializer)?;
            let bytes = const_hex::decode(&s).map_err(serde::de::Error::custom)?;
            let key_bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| serde::de::Error::custom("Invalid byte length: expected 32 bytes"))?;

            let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&key_bytes)
                .map_err(serde::de::Error::custom)?;

            Ok(Self(verifying_key))
        } else {
            // For binary: deserialize from bytes
            Ok(Self(ed25519_dalek::VerifyingKey::deserialize(
                deserializer,
            )?))
        }
    }
}

impl PartialOrd for ProviderId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ProviderId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_bytes().cmp(other.0.as_bytes())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeclarationId(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActivityId(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declaration {
    pub service_type: ServiceType,
    pub provider_id: ProviderId,
    pub locked_note_id: NoteId,
    pub locators: Vec<Locator>,
    pub zk_id: PublicKey,
    pub created: BlockNumber,
    pub active: BlockNumber,
    pub withdrawn: Option<BlockNumber>,
    pub nonce: Nonce,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderInfo {
    pub locators: Vec<Locator>,
    pub zk_id: PublicKey,
}

impl Declaration {
    #[must_use]
    pub fn new(block_number: BlockNumber, declaration_msg: &DeclarationMessage) -> Self {
        Self {
            service_type: declaration_msg.service_type,
            provider_id: declaration_msg.provider_id,
            locked_note_id: declaration_msg.locked_note_id,
            locators: declaration_msg.locators.clone(),
            zk_id: declaration_msg.zk_id,
            created: block_number,
            active: block_number,
            withdrawn: None,
            nonce: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct DeclarationMessage {
    pub service_type: ServiceType,
    pub locators: Vec<Locator>,
    pub provider_id: ProviderId,
    pub zk_id: PublicKey,
    pub locked_note_id: NoteId,
}

impl DeclarationMessage {
    #[must_use]
    pub fn id(&self) -> DeclarationId {
        let mut hasher = Blake2b::new();
        let service = match self.service_type {
            ServiceType::BlendNetwork => "BN",
            ServiceType::DataAvailability => "DA",
        };

        // From the
        // [spec](https://www.notion.so/nomos-tech/Service-Declaration-Protocol-Specification-1fd261aa09df819ca9f8eb2bdfd4ec1dw):
        // declaration_id = Hash(service||provider_id||zk_id||locators)
        hasher.update(service.as_bytes());
        hasher.update(self.provider_id.0);
        for number in self.zk_id.as_fr().0.0 {
            hasher.update(number.to_le_bytes());
        }
        for locator in &self.locators {
            hasher.update(locator.0.as_ref());
        }

        DeclarationId(hasher.finalize().into())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct WithdrawMessage {
    pub declaration_id: DeclarationId,
    pub locked_note_id: NoteId,
    pub nonce: Nonce,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ActiveMessage {
    pub declaration_id: DeclarationId,
    pub nonce: Nonce,
    pub metadata: ActivityMetadata,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DaActivityProof {
    pub current_session: SessionNumber,
    pub previous_session_opinions: Vec<u8>,
    pub current_session_opinions: Vec<u8>,
}

impl DaActivityProof {
    #[must_use]
    pub fn to_metadata_bytes(&self) -> Vec<u8> {
        let total_size = 2 // type + version byte 
            + size_of::<SessionNumber>()
            + size_of::<DaMetadataLengthPrefix>() // previous_session_opinions_length
            + self.previous_session_opinions.len()
            + size_of::<DaMetadataLengthPrefix>() // current_session_opinions_length
            + self.current_session_opinions.len();

        let mut bytes = Vec::with_capacity(total_size);
        bytes.push(ACTIVE_METADATA_DA_TYPE);
        bytes.push(DA_ACTIVE_METADATA_VERSION_BYTE);
        bytes.extend(&self.current_session.to_le_bytes());

        // Encode previous opinions with length prefix
        bytes.extend(
            &(self.previous_session_opinions.len() as DaMetadataLengthPrefix).to_le_bytes(),
        );
        bytes.extend(&self.previous_session_opinions);

        // Encode current opinions with length prefix
        bytes
            .extend(&(self.current_session_opinions.len() as DaMetadataLengthPrefix).to_le_bytes());
        bytes.extend(&self.current_session_opinions);

        bytes
    }

    /// Parse metadata bytes using nom combinators
    pub fn from_metadata_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        if bytes.is_empty() {
            return Err("empty metadata bytes".to_owned().into());
        }

        if bytes.len() < DA_MIN_METADATA_SIZE {
            return Err(format!(
                "Metadata too short: got {} bytes, expected at least {}",
                bytes.len(),
                DA_MIN_METADATA_SIZE
            )
            .into());
        }

        let (_, proof) =
            parse_da_activity_proof(bytes).map_err(|e| format!("Failed to parse metadata: {e}"))?;

        Ok(proof)
    }
}

fn parse_da_activity_proof(input: &[u8]) -> IResult<&[u8], DaActivityProof> {
    let (input, metadata_type) = nom_u8(input)?;
    if metadata_type != ACTIVE_METADATA_DA_TYPE {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        )));
    }

    let (input, version) = nom_u8(input)?;
    if version != DA_ACTIVE_METADATA_VERSION_BYTE {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        )));
    }

    let (input, current_session) = parse_session_number(input)?;
    let (input, previous_session_opinions) = parse_length_prefixed_bytes(input)?;
    let (input, current_session_opinions) = parse_length_prefixed_bytes(input)?;

    if !input.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Eof,
        )));
    }

    Ok((
        input,
        DaActivityProof {
            current_session,
            previous_session_opinions,
            current_session_opinions,
        },
    ))
}

fn parse_session_number(input: &[u8]) -> IResult<&[u8], SessionNumber> {
    let (input, bytes) = take(size_of::<SessionNumber>()).parse(input)?;
    let session_bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Fail)))?;
    Ok((input, SessionNumber::from_le_bytes(session_bytes)))
}

/// Parse length-prefixed byte vector: u32 length + data
fn parse_length_prefixed_bytes(input: &[u8]) -> IResult<&[u8], Vec<u8>> {
    let (input, len) = le_u32(input)?;
    let (input, data) = take(len as usize).parse(input)?;
    Ok((input, data.to_vec()))
}

/// A Blend activity proof for a session, made of the blending token
/// that has the smallest Hamming distance satisfying the activity threshold.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct BlendActivityProof {
    session_number: SessionNumber,
    token: BlendingToken,
}

const BLEND_ACTIVE_METADATA_VERSION_BYTE: u8 = 0x01;

impl BlendActivityProof {
    #[must_use]
    pub const fn new(session_number: SessionNumber, token: BlendingToken) -> Self {
        Self {
            session_number,
            token,
        }
    }

    #[must_use]
    pub const fn token(&self) -> &BlendingToken {
        &self.token
    }

    #[must_use]
    fn to_metadata_bytes(&self) -> Vec<u8> {
        let session_number = self.session_number.to_le_bytes();
        let proof_of_quota: [u8; PROOF_OF_QUOTA_SIZE] = self.token.proof_of_quota().into();
        let proof_of_selection: [u8; PROOF_OF_SELECTION_SIZE] =
            self.token.proof_of_selection().into();

        let total_size = 2 // type + version byte
            + session_number.len()
            + proof_of_quota.len()
            + proof_of_selection.len();

        let mut bytes = Vec::with_capacity(total_size);
        bytes.push(ACTIVE_METADATA_BLEND_TYPE);
        bytes.push(BLEND_ACTIVE_METADATA_VERSION_BYTE);
        bytes.extend(&session_number);
        bytes.extend(&proof_of_quota);
        bytes.extend(&proof_of_selection);
        bytes
    }

    /// Parse metadata bytes using nom combinators
    fn from_metadata_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(parse_blend_activity_proof(bytes)
            .map_err(|e| format!("Failed to parse metadata: {e}"))?
            .1)
    }
}

fn parse_blend_activity_proof(input: &[u8]) -> IResult<&[u8], BlendActivityProof> {
    let (input, metadata_type) = nom_u8(input)?;
    if metadata_type != ACTIVE_METADATA_BLEND_TYPE {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        )));
    }

    let (input, version) = nom_u8(input)?;
    if version != BLEND_ACTIVE_METADATA_VERSION_BYTE {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        )));
    }

    let (input, session) = parse_session_number(input)?;

    let (input, proof_of_quota) = parse_const_size_bytes::<PROOF_OF_QUOTA_SIZE>(input)?;
    let proof_of_quota = proof_of_quota.try_into().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Verify))
    })?;

    let (input, proof_of_selection) = parse_const_size_bytes::<PROOF_OF_SELECTION_SIZE>(input)?;
    let proof_of_selection = proof_of_selection.try_into().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Verify))
    })?;

    if !input.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Eof,
        )));
    }

    Ok((
        input,
        BlendActivityProof::new(
            session,
            BlendingToken::new(proof_of_quota, proof_of_selection),
        ),
    ))
}

fn parse_const_size_bytes<const N: usize>(input: &[u8]) -> IResult<&[u8], [u8; N]> {
    let (input, data) = take(N).parse(input)?;
    let data: [u8; N] = data
        .try_into()
        .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Fail)))?;
    Ok((input, data))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ActivityMetadata {
    DataAvailability(DaActivityProof),
    Blend(BlendActivityProof),
}

impl ActivityMetadata {
    #[must_use]
    pub fn to_metadata_bytes(&self) -> Vec<u8> {
        match self {
            Self::DataAvailability(proof) => proof.to_metadata_bytes(),
            Self::Blend(proof) => proof.to_metadata_bytes(),
        }
    }

    pub fn from_metadata_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        if bytes.is_empty() {
            return Err("empty metadata bytes".to_owned().into());
        }

        // Read metadata type byte to determine variant
        let metadata_type = bytes[0];

        match metadata_type {
            ACTIVE_METADATA_DA_TYPE => {
                let proof_opt = DaActivityProof::from_metadata_bytes(bytes)?;
                Ok(Self::DataAvailability(proof_opt))
            }
            ACTIVE_METADATA_BLEND_TYPE => {
                let proof_opt = BlendActivityProof::from_metadata_bytes(bytes)?;
                Ok(Self::Blend(proof_opt))
            }
            _ => Err(format!("Unknown metadata type: {metadata_type:#x}").into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        blend::{ProofOfQuota, ProofOfSelection},
        mantle::encoding::encode_sdp_active,
    };

    #[test]
    fn test_da_activity_proof_roundtrip_empty_opinions() {
        let proof = DaActivityProof {
            current_session: 42,
            previous_session_opinions: vec![],
            current_session_opinions: vec![],
        };

        let bytes = proof.to_metadata_bytes();
        let decoded = DaActivityProof::from_metadata_bytes(&bytes).unwrap();
        assert_eq!(proof, decoded);
    }

    #[test]
    fn test_da_activity_proof_roundtrip_with_data() {
        let proof = DaActivityProof {
            current_session: 123,
            previous_session_opinions: vec![0xFF, 0xAA, 0x55],
            current_session_opinions: vec![0x01, 0x02, 0x03, 0x04],
        };

        let bytes = proof.to_metadata_bytes();
        let decoded = DaActivityProof::from_metadata_bytes(&bytes).unwrap();

        assert_eq!(proof, decoded);
    }

    #[test]
    fn test_da_activity_proof_byte_format() {
        let proof = DaActivityProof {
            current_session: 1,
            previous_session_opinions: vec![0xAA],
            current_session_opinions: vec![0xBB, 0xCC],
        };

        let bytes = proof.to_metadata_bytes();

        // Verify format: type(1) + version(1) + session(8) + prev_len(4) + prev_data +
        // curr_len(4) + curr_data
        assert_eq!(bytes[0], ACTIVE_METADATA_DA_TYPE); // version
        assert_eq!(bytes[1], DA_ACTIVE_METADATA_VERSION_BYTE); // version

        // Session number (little-endian u64)
        let session_bytes: [u8; 8] = bytes[2..10].try_into().unwrap();
        assert_eq!(u64::from_le_bytes(session_bytes), 1);

        // Previous opinions length
        let prev_len_bytes: [u8; 4] = bytes[10..14].try_into().unwrap();
        assert_eq!(u32::from_le_bytes(prev_len_bytes), 1);

        // Previous opinions data
        assert_eq!(bytes[14], 0xAA);

        // Current opinions length
        let curr_len_bytes: [u8; 4] = bytes[15..19].try_into().unwrap();
        assert_eq!(u32::from_le_bytes(curr_len_bytes), 2);

        // Current opinions data
        assert_eq!(bytes[19], 0xBB);
        assert_eq!(bytes[20], 0xCC);

        // Total length check
        assert_eq!(bytes.len(), 21); // 1 + 1 + 8 + 4 + 1 + 4 + 2
    }

    #[test]
    fn test_da_activity_proof_empty_metadata() {
        let proof = DaActivityProof {
            current_session: 999,
            previous_session_opinions: vec![],
            current_session_opinions: vec![],
        };

        let bytes = proof.to_metadata_bytes();

        // Check that empty vectors are encoded with zero length
        let prev_len_bytes: [u8; 4] = bytes[9..13].try_into().unwrap();
        assert_eq!(u32::from_le_bytes(prev_len_bytes), 0);

        let curr_len_bytes: [u8; 4] = bytes[13..17].try_into().unwrap();
        assert_eq!(u32::from_le_bytes(curr_len_bytes), 0);

        // Total: version(1) + session(8) + prev_len(4) + curr_len(4) = 17 bytes
        assert_eq!(bytes.len(), DA_MIN_METADATA_SIZE);

        let decoded = DaActivityProof::from_metadata_bytes(&bytes).unwrap();
        assert_eq!(proof, decoded);
    }

    #[test]
    fn test_da_activity_proof_large_opinions() {
        let proof = DaActivityProof {
            current_session: u64::MAX,
            previous_session_opinions: vec![0xFF; 1000],
            current_session_opinions: vec![0xAA; 2000],
        };

        let bytes = proof.to_metadata_bytes();
        let decoded = DaActivityProof::from_metadata_bytes(&bytes).unwrap();

        assert_eq!(proof, decoded);
        assert_eq!(decoded.previous_session_opinions.len(), 1000);
        assert_eq!(decoded.current_session_opinions.len(), 2000);
    }

    #[test]
    fn test_da_activity_proof_invalid_version() {
        let proof = DaActivityProof {
            current_session: 1,
            previous_session_opinions: vec![0xAA],
            current_session_opinions: vec![0xBB, 0xCC],
        };
        let mut bytes = proof.to_metadata_bytes();
        bytes[1] = 0x99; // Invalid version

        let result = DaActivityProof::from_metadata_bytes(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }

    #[test]
    fn test_da_activity_proof_too_short() {
        let bytes = vec![DA_ACTIVE_METADATA_VERSION_BYTE, 0x01, 0x02]; // Only 3 bytes

        let result = DaActivityProof::from_metadata_bytes(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too short"));
    }

    #[test]
    fn test_da_activity_proof_truncated_data() {
        let mut bytes = vec![DA_ACTIVE_METADATA_VERSION_BYTE];
        bytes.extend(&1u64.to_le_bytes()); // session
        bytes.extend(&5u32.to_le_bytes()); // prev len = 5
        bytes.extend(&[0xAA, 0xBB]); // Only 2 bytes instead of 5

        let result = DaActivityProof::from_metadata_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_da_activity_proof_extra_bytes() {
        let proof = DaActivityProof {
            current_session: 10,
            previous_session_opinions: vec![0x11],
            current_session_opinions: vec![0x22],
        };

        let mut bytes = proof.to_metadata_bytes();
        bytes.push(0xFF); // Extra byte

        let result = DaActivityProof::from_metadata_bytes(&bytes);
        assert!(result.is_err()); // Should fail due to extra bytes
    }

    #[test]
    fn test_blend_activity_proof_roundtrip() {
        let proof = BlendActivityProof::new(
            10,
            BlendingToken::new(
                ProofOfQuota::from_bytes_unchecked([0; _]),
                ProofOfSelection::from_bytes_unchecked([1; _]),
            ),
        );

        let bytes = proof.to_metadata_bytes();
        let decoded = BlendActivityProof::from_metadata_bytes(&bytes).unwrap();

        assert_eq!(proof, decoded);
    }

    #[test]
    fn test_blend_activity_proof_invalid_version() {
        let proof = BlendActivityProof::new(
            10,
            BlendingToken::new(
                ProofOfQuota::from_bytes_unchecked([0; _]),
                ProofOfSelection::from_bytes_unchecked([1; _]),
            ),
        );
        let mut bytes = proof.to_metadata_bytes();
        bytes[0] = 0x99; // Invalid version

        let result = BlendActivityProof::from_metadata_bytes(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }

    #[test]
    fn test_blend_activity_proof_too_short() {
        let bytes = vec![BLEND_ACTIVE_METADATA_VERSION_BYTE, 0x01, 0x02]; // Only 3 bytes

        let result = BlendActivityProof::from_metadata_bytes(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Eof"));
    }

    #[test]
    fn test_blend_activity_proof_too_long() {
        let proof = BlendActivityProof::new(
            10,
            BlendingToken::new(
                ProofOfQuota::from_bytes_unchecked([0; _]),
                ProofOfSelection::from_bytes_unchecked([1; _]),
            ),
        );
        let mut bytes = proof.to_metadata_bytes();
        bytes.push(0xFF); // An extra byte

        let result = BlendActivityProof::from_metadata_bytes(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Eof"));
    }

    #[test]
    fn test_da_activity_metadata_roundtrip() {
        let proof = DaActivityProof {
            current_session: 456,
            previous_session_opinions: vec![0x12, 0x34],
            current_session_opinions: vec![0x56, 0x78, 0x9A],
        };
        let metadata = ActivityMetadata::DataAvailability(proof.clone());

        let bytes = metadata.to_metadata_bytes();
        let decoded = ActivityMetadata::from_metadata_bytes(&bytes).unwrap();

        assert_eq!(metadata, decoded);

        let ActivityMetadata::DataAvailability(decoded_proof) = decoded else {
            panic!("Unexpected ActivityMetadata variant");
        };
        assert_eq!(proof, decoded_proof);
    }

    #[test]
    fn test_blend_activity_metadata_roundtrip() {
        let proof = BlendActivityProof::new(
            10,
            BlendingToken::new(
                ProofOfQuota::from_bytes_unchecked([0; _]),
                ProofOfSelection::from_bytes_unchecked([1; _]),
            ),
        );
        let metadata = ActivityMetadata::Blend(proof.clone());

        let bytes = metadata.to_metadata_bytes();
        let decoded = ActivityMetadata::from_metadata_bytes(&bytes).unwrap();

        assert_eq!(metadata, decoded);

        let ActivityMetadata::Blend(decoded_proof) = decoded else {
            panic!("Unexpected ActivityMetadata variant");
        };
        assert_eq!(proof, decoded_proof);
    }

    #[test]
    fn test_activity_metadata_empty_bytes() {
        let result = ActivityMetadata::from_metadata_bytes(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_activity_metadata_unknown_type() {
        let bytes = vec![0xFF]; // Unknown type
        let result = ActivityMetadata::from_metadata_bytes(&bytes);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown metadata type")
        );
    }

    #[test]
    fn test_active_message_with_metadata() {
        let proof = DaActivityProof {
            current_session: 100,
            previous_session_opinions: vec![0x01, 0x02],
            current_session_opinions: vec![0x03, 0x04],
        };
        let metadata = ActivityMetadata::DataAvailability(proof);

        let message = ActiveMessage {
            declaration_id: DeclarationId([0xAA; 32]),
            nonce: 42,
            metadata,
        };

        let bytes = encode_sdp_active(&message);

        // Verify structure: declaration_id(32) + nonce(8) + metadata
        assert!(bytes.len() > 40); // At least 32 + 8 + some metadata

        // Verify declaration_id
        assert_eq!(&bytes[..32], &[0xAA; 32]);

        // Verify nonce
        let nonce_bytes: [u8; 8] = bytes[32..40].try_into().unwrap();
        assert_eq!(u64::from_le_bytes(nonce_bytes), 42);
    }
}
