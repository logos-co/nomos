pub mod state;

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::Hash,
    ops::Deref,
};

use blake2::{Blake2b, Digest as _};
use bytes::{Bytes, BytesMut};
use groth16::{Fr, serde::serde_fr};
use multiaddr::Multiaddr;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use state::TransientDeclarationState;
use strum::EnumIter;

use crate::{
    block::{BlockNumber, SessionNumber},
    mantle::NoteId,
};

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

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct DeclarationInfo {
    pub id: DeclarationId,
    pub provider_id: ProviderId,
    pub service: ServiceType,
    pub locators: Vec<Locator>,
    pub zk_id: ZkPublicKey,
    pub locked_note_id: NoteId,
}

impl DeclarationInfo {
    #[must_use]
    pub fn new(msg: DeclarationMessage) -> Self {
        Self {
            id: msg.declaration_id(),
            provider_id: msg.provider_id,
            service: msg.service_type,
            locators: msg.locators,
            zk_id: msg.zk_id,
            locked_note_id: msg.locked_note_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclarationState {
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

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub locators: Vec<Locator>,
    pub zk_id: ZkPublicKey,
}

impl DeclarationState {
    #[must_use]
    pub const fn new(
        block_number: BlockNumber,
        provider_id: ProviderId,
        locators: Vec<Locator>,
        service_type: ServiceType,
        locked_note_id: NoteId,
        zk_id: ZkPublicKey,
    ) -> Self {
        Self {
            service_type,
            locked_note_id,
            zk_id,
            provider_id,
            locators,
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
    pub fn declaration_id(&self) -> DeclarationId {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ActiveMessage<Metadata> {
    pub declaration_id: DeclarationId,
    pub nonce: Nonce,
    pub metadata: Option<Metadata>,
}

impl<Metadata: AsRef<[u8]>> ActiveMessage<Metadata> {
    #[must_use]
    pub fn payload_bytes(&self) -> Bytes {
        let mut buff = BytesMut::new();
        buff.extend_from_slice(self.declaration_id.0.as_ref());
        buff.extend_from_slice(&(self.nonce.to_le_bytes()));
        if let Some(metadata) = &self.metadata {
            buff.extend_from_slice(metadata.as_ref());
        }
        buff.freeze()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EventType {
    Declaration,
    Activity,
    Withdrawal,
}

pub struct Event {
    pub provider_id: ProviderId,
    pub event_type: EventType,
    pub service_type: ServiceType,
    pub timestamp: BlockNumber,
}

pub enum SdpMessage<Metadata> {
    Declare(Box<DeclarationMessage>),
    Activity(ActiveMessage<Metadata>),
    Withdraw(WithdrawMessage),
}

impl<Metadata> SdpMessage<Metadata> {
    #[must_use]
    pub fn declaration_id(&self) -> DeclarationId {
        match self {
            Self::Declare(message) => message.declaration_id(),
            Self::Activity(message) => message.declaration_id,
            Self::Withdraw(message) => message.declaration_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FinalizedDeclarationState {
    Active,
    Inactive,
    Withdrawn,
}

impl<D> From<&TransientDeclarationState<D>> for FinalizedDeclarationState
where
    D: Deref<Target = DeclarationState>,
{
    fn from(transient: &TransientDeclarationState<D>) -> Self {
        match transient {
            TransientDeclarationState::Active(_) => Self::Active,
            TransientDeclarationState::Inactive(_) => Self::Inactive,
            TransientDeclarationState::Withdrawn(_) => Self::Withdrawn,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizedBlockEvent {
    pub block_number: BlockNumber,
    pub updates: Vec<FinalizedBlockEventUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizedBlockEventUpdate {
    pub service_type: ServiceType,
    pub provider_id: ProviderId,
    pub state: FinalizedDeclarationState,
    pub locators: BTreeSet<Locator>,
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct Session {
    pub session_number: SessionNumber,
    pub declarations: HashSet<DeclarationId>,
}

impl Session {
    pub fn update(&mut self, declaration_id: DeclarationId, state: &FinalizedDeclarationState) {
        match state {
            FinalizedDeclarationState::Active => {
                self.declarations.insert(declaration_id);
            }
            FinalizedDeclarationState::Inactive | FinalizedDeclarationState::Withdrawn => {
                self.declarations.remove(&declaration_id);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionUpdate {
    pub session_number: SessionNumber,
    pub providers: HashMap<ProviderId, ProviderInfo>,
}
