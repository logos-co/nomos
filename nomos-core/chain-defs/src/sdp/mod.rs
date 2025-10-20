use std::hash::Hash;

use blake2::{Blake2b, Digest as _};
use bytes::{Bytes, BytesMut};
use groth16::{Fr, serde::serde_fr};
use multiaddr::Multiaddr;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::EnumIter;

use crate::{block::BlockNumber, mantle::NoteId};

pub type SessionNumber = u64;
pub type StakeThreshold = u64;

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

const METADATA_VERSION_BYTE: u8 = 0x01;

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
            + 4 // previous_session_opinions_length (u32)
            + self.previous_session_opinions.len()
            + 4 // current_session_opinions_length (u32)
            + self.current_session_opinions.len();

        let mut bytes = Vec::with_capacity(total_size);
        bytes.push(METADATA_VERSION_BYTE);
        bytes.extend(&self.current_session.to_le_bytes());

        // Encode previous opinions with length prefix
        bytes.extend(&(self.previous_session_opinions.len() as u32).to_le_bytes());
        bytes.extend(&self.previous_session_opinions);

        // Encode current opinions with length prefix
        bytes.extend(&(self.current_session_opinions.len() as u32).to_le_bytes());
        bytes.extend(&self.current_session_opinions);

        bytes
    }

    pub fn from_metadata_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        // Minimum size: version(1) + session(8) + prev_len(4) + curr_len(4) = 17
        if bytes.len() < 17 {
            return Err("Metadata too short".into());
        }

        let mut offset = 0;

        // Check version byte
        let version = bytes[offset];
        if version != METADATA_VERSION_BYTE {
            return Err(format!(
                "Invalid version byte: {version:#x}, expected {METADATA_VERSION_BYTE:#x}"
            )
            .into());
        }
        offset += 1;

        // Parse session number (8 bytes)
        let session_bytes: [u8; 8] = bytes[offset..offset + 8]
            .try_into()
            .map_err(|_| "Failed to read session number")?;
        let current_session = SessionNumber::from_le_bytes(session_bytes);
        offset += 8;

        // Parse previous session opinions length (4 bytes)
        let prev_len_bytes: [u8; 4] = bytes[offset..offset + 4]
            .try_into()
            .map_err(|_| "Failed to read previous opinions length")?;
        let prev_len = u32::from_le_bytes(prev_len_bytes) as usize;
        offset += 4;

        // Read previous session opinions data
        if offset + prev_len > bytes.len() {
            return Err("Previous opinions length exceeds remaining bytes".into());
        }
        let previous_session_opinions = bytes[offset..offset + prev_len].to_vec();
        offset += prev_len;

        // Parse current session opinions length (4 bytes)
        if offset + 4 > bytes.len() {
            return Err("Not enough bytes for current opinions length".into());
        }
        let curr_len_bytes: [u8; 4] = bytes[offset..offset + 4]
            .try_into()
            .map_err(|_| "Failed to read current opinions length")?;
        let curr_len = u32::from_le_bytes(curr_len_bytes) as usize;
        offset += 4;

        // Read current session opinions data
        if offset + curr_len != bytes.len() {
            return Err(format!(
                "Current opinions length mismatch: expected {}, got {} remaining bytes",
                curr_len,
                bytes.len() - offset
            )
            .into());
        }
        let current_session_opinions = bytes[offset..offset + curr_len].to_vec();

        Ok(Self {
            current_session,
            previous_session_opinions,
            current_session_opinions,
        })
    }
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

    pub fn from_metadata_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        if bytes.is_empty() {
            return Err("Empty metadata bytes".into());
        }

        // Read version byte to determine variant
        let version = bytes[0];

        match version {
            METADATA_VERSION_BYTE => {
                let proof = DaActivityProof::from_metadata_bytes(bytes)?;
                Ok(Self::DataAvailability(proof))
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
