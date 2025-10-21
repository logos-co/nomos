use std::hash::Hash;

use blake2::{Blake2b, Digest as _};
use bytes::{Bytes, BytesMut};
use groth16::{Fr, serde::serde_fr};
use multiaddr::Multiaddr;
use nom::{
    IResult, Parser as _,
    bytes::complete::take,
    number::complete::{le_u32, u8 as nom_u8},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::EnumIter;

use crate::{block::BlockNumber, mantle::NoteId};

pub type SessionNumber = u64;
pub type StakeThreshold = u64;

const DA_ACTIVE_METADATA_VERSION_BYTE: u8 = 0x01;
type DaMetadataLengthPrefix = u32;
const DA_MIN_METADATA_SIZE: usize =
    1 + size_of::<SessionNumber>() + size_of::<DaMetadataLengthPrefix>() * 2;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct DeclarationId(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActivityId(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ZkPublicKey(#[serde(with = "serde_fr")] pub Fr);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declaration {
    pub service_type: ServiceType,
    pub provider_id: ProviderId,
    pub locked_note_id: NoteId,
    pub locators: Vec<Locator>,
    pub zk_id: ZkPublicKey,
    pub created: BlockNumber,
    pub active: BlockNumber,
    pub withdrawn: Option<BlockNumber>,
    pub nonce: Nonce,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderInfo {
    pub locators: Vec<Locator>,
    pub zk_id: ZkPublicKey,
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
    pub zk_id: ZkPublicKey,
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
        for number in self.zk_id.0.0.0 {
            hasher.update(number.to_le_bytes());
        }
        for locator in &self.locators {
            hasher.update(locator.0.as_ref());
        }

        DeclarationId(hasher.finalize().into())
    }

    #[must_use]
    pub fn payload_bytes(&self) -> Bytes {
        let mut buff = BytesMut::new();
        buff.extend_from_slice(self.service_type.as_ref().as_bytes());
        for locator in &self.locators {
            buff.extend_from_slice(locator.0.as_ref());
        }
        buff.extend_from_slice(self.provider_id.0.as_ref());
        buff.extend(self.zk_id.0.0.0.iter().flat_map(|n| n.to_le_bytes()));
        buff.freeze()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct WithdrawMessage {
    pub declaration_id: DeclarationId,
    pub nonce: Nonce,
}

impl WithdrawMessage {
    #[must_use]
    pub fn payload_bytes(&self) -> Bytes {
        let mut buff = BytesMut::new();
        buff.extend_from_slice(self.declaration_id.0.as_ref());
        buff.extend_from_slice(&(self.nonce.to_le_bytes()));
        buff.freeze()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ActiveMessage {
    pub declaration_id: DeclarationId,
    pub nonce: Nonce,
    pub metadata: Option<ActivityMetadata>,
}

impl ActiveMessage {
    #[must_use]
    pub fn payload_bytes(&self) -> Bytes {
        let mut buff = BytesMut::new();
        buff.extend_from_slice(self.declaration_id.0.as_ref());
        buff.extend_from_slice(&(self.nonce.to_le_bytes()));
        if let Some(metadata) = &self.metadata {
            buff.extend_from_slice(&metadata.to_metadata_bytes());
        }
        buff.freeze()
    }
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
        let total_size = 1 // version byte
            + size_of::<SessionNumber>()
            + size_of::<DaMetadataLengthPrefix>() // previous_session_opinions_length
            + self.previous_session_opinions.len()
            + size_of::<DaMetadataLengthPrefix>() // current_session_opinions_length
            + self.current_session_opinions.len();

        let mut bytes = Vec::with_capacity(total_size);
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
    pub fn from_metadata_bytes(bytes: &[u8]) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        if bytes.is_empty() {
            return Ok(None);
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

        Ok(Some(proof))
    }
}

fn parse_da_activity_proof(input: &[u8]) -> IResult<&[u8], DaActivityProof> {
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ActivityMetadata {
    DataAvailability(DaActivityProof),
    // Blend will add: Blend(BlendActivityProof),
}

impl ActivityMetadata {
    #[must_use]
    pub fn to_metadata_bytes(&self) -> Vec<u8> {
        match self {
            Self::DataAvailability(proof) => proof.to_metadata_bytes(),
            // Future: ActivityMetadata::Blend(proof) => proof.to_metadata_bytes(),
        }
    }

    pub fn from_metadata_bytes(bytes: &[u8]) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        if bytes.is_empty() {
            return Ok(None);
        }

        // Read version byte to determine variant
        let version = bytes[0];

        match version {
            DA_ACTIVE_METADATA_VERSION_BYTE => {
                let proof_opt = DaActivityProof::from_metadata_bytes(bytes)?;
                Ok(proof_opt.map(Self::DataAvailability))
            }
            _ => Err(format!("Unknown metadata version: {version:#x}").into()),
        }
    }
}

pub enum SdpMessage {
    Declare(Box<DeclarationMessage>),
    Activity(ActiveMessage),
    Withdraw(WithdrawMessage),
}

impl SdpMessage {
    #[must_use]
    pub fn declaration_id(&self) -> DeclarationId {
        match self {
            Self::Declare(message) => message.id(),
            Self::Activity(message) => message.declaration_id,
            Self::Withdraw(message) => message.declaration_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_da_activity_proof_roundtrip_empty_opinions() {
        let proof = DaActivityProof {
            current_session: 42,
            previous_session_opinions: vec![],
            current_session_opinions: vec![],
        };

        let bytes = proof.to_metadata_bytes();
        let decoded = DaActivityProof::from_metadata_bytes(&bytes)
            .unwrap()
            .expect("Should have activity proof metadata");

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
        let decoded = DaActivityProof::from_metadata_bytes(&bytes)
            .unwrap()
            .expect("Should have activity proof metadata");

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

        // Verify format: version(1) + session(8) + prev_len(4) + prev_data +
        // curr_len(4) + curr_data
        assert_eq!(bytes[0], DA_ACTIVE_METADATA_VERSION_BYTE); // version

        // Session number (little-endian u64)
        let session_bytes: [u8; 8] = bytes[1..9].try_into().unwrap();
        assert_eq!(u64::from_le_bytes(session_bytes), 1);

        // Previous opinions length
        let prev_len_bytes: [u8; 4] = bytes[9..13].try_into().unwrap();
        assert_eq!(u32::from_le_bytes(prev_len_bytes), 1);

        // Previous opinions data
        assert_eq!(bytes[13], 0xAA);

        // Current opinions length
        let curr_len_bytes: [u8; 4] = bytes[14..18].try_into().unwrap();
        assert_eq!(u32::from_le_bytes(curr_len_bytes), 2);

        // Current opinions data
        assert_eq!(bytes[18], 0xBB);
        assert_eq!(bytes[19], 0xCC);

        // Total length check
        assert_eq!(bytes.len(), 20); // 1 + 8 + 4 + 1 + 4 + 2
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

        let decoded = DaActivityProof::from_metadata_bytes(&bytes)
            .unwrap()
            .expect("Should have activity proof metadata");
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
        let decoded = DaActivityProof::from_metadata_bytes(&bytes)
            .unwrap()
            .expect("Should have activity proof metadata");

        assert_eq!(proof, decoded);
        assert_eq!(decoded.previous_session_opinions.len(), 1000);
        assert_eq!(decoded.current_session_opinions.len(), 2000);
    }

    #[test]
    fn test_da_activity_proof_invalid_version() {
        let mut bytes = vec![0x99]; // Wrong version
        bytes.extend(&1u64.to_le_bytes()); // session
        bytes.extend(&0u32.to_le_bytes()); // prev len
        bytes.extend(&0u32.to_le_bytes()); // curr len

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
    fn test_activity_metadata_roundtrip() {
        let proof = DaActivityProof {
            current_session: 456,
            previous_session_opinions: vec![0x12, 0x34],
            current_session_opinions: vec![0x56, 0x78, 0x9A],
        };
        let metadata = ActivityMetadata::DataAvailability(proof.clone());

        let bytes = metadata.to_metadata_bytes();
        let decoded = ActivityMetadata::from_metadata_bytes(&bytes)
            .unwrap()
            .expect("Should have activity proof metadata");

        assert_eq!(metadata, decoded);

        let ActivityMetadata::DataAvailability(decoded_proof) = decoded;
        assert_eq!(proof, decoded_proof);
    }

    #[test]
    fn test_activity_metadata_empty_bytes() {
        let result = ActivityMetadata::from_metadata_bytes(&[]).expect("should be OK");
        assert!(result.is_none());
    }

    #[test]
    fn test_activity_metadata_unknown_version() {
        let bytes = vec![0xFF]; // Unknown version
        let result = ActivityMetadata::from_metadata_bytes(&bytes);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown metadata version")
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
            metadata: Some(metadata),
        };

        let bytes = message.payload_bytes();

        // Verify structure: declaration_id(32) + nonce(8) + metadata
        assert!(bytes.len() > 40); // At least 32 + 8 + some metadata

        // Verify declaration_id
        assert_eq!(&bytes[..32], &[0xAA; 32]);

        // Verify nonce
        let nonce_bytes: [u8; 8] = bytes[32..40].try_into().unwrap();
        assert_eq!(u64::from_le_bytes(nonce_bytes), 42);
    }

    #[test]
    fn test_active_message_without_metadata() {
        let message = ActiveMessage {
            declaration_id: DeclarationId([0xBB; 32]),
            nonce: 123,
            metadata: None,
        };

        let bytes = message.payload_bytes();

        // Should only have declaration_id + nonce
        assert_eq!(bytes.len(), 40); // 32 + 8
    }
}
