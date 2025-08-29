pub mod state;

use std::{collections::BTreeSet, hash::Hash};

use blake2::{Blake2b, Digest as _};
use bytes::{Bytes, BytesMut};
use multiaddr::Multiaddr;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::block::BlockNumber;

pub type StakeThreshold = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct MinStake {
    pub threshold: StakeThreshold,
    pub timestamp: BlockNumber,
}

#[derive(Clone, Debug)]
pub struct ServiceParameters {
    pub lock_period: u64,
    pub inactivity_period: u64,
    pub retention_period: u64,
    pub timestamp: BlockNumber,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ServiceType {
    #[serde(rename = "BN")]
    BlendNetwork,
    #[serde(rename = "DA")]
    DataAvailability,
    #[serde(rename = "EX")]
    ExecutorNetwork,
}

impl AsRef<str> for ServiceType {
    fn as_ref(&self) -> &str {
        match self {
            Self::BlendNetwork => "BN",
            Self::DataAvailability => "DA",
            Self::ExecutorNetwork => "EX",
        }
    }
}

pub type Nonce = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ProviderId(pub [u8; 32]);

impl Serialize for ProviderId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            // For JSON: serialize as hex string
            const_hex::encode(self.0).serialize(serializer)
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
            if bytes.len() != 32 {
                return Err(serde::de::Error::custom("Invalid byte length"));
            }
            Ok(Self(bytes.try_into().unwrap()))
        } else {
            // For binary: deserialize from bytes
            Ok(Self(<[u8; 32]>::deserialize(deserializer)?))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct DeclarationId(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActivityId(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ZkPublicKey(pub [u8; 32]);

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct DeclarationInfo {
    pub id: DeclarationId,
    pub provider_id: ProviderId,
    pub service: ServiceType,
    pub locators: Vec<Locator>,
    pub zk_id: ZkPublicKey,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclarationState {
    pub created: BlockNumber,
    pub active: BlockNumber,
    pub withdrawn: Option<BlockNumber>,
    pub nonce: Nonce,
}

impl DeclarationState {
    #[must_use]
    pub const fn new(block_number: BlockNumber) -> Self {
        Self {
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
}

impl DeclarationMessage {
    #[must_use]
    pub fn declaration_id(&self) -> DeclarationId {
        let mut hasher = Blake2b::new();
        let service = match self.service_type {
            ServiceType::BlendNetwork => "BN",
            ServiceType::DataAvailability => "DA",
            ServiceType::ExecutorNetwork => "EX",
        };

        // From the
        // [spec](https://www.notion.so/nomos-tech/Service-Declaration-Protocol-Specification-1fd261aa09df819ca9f8eb2bdfd4ec1dw):
        // declaration_id = Hash(service||provider_id||zk_id||locators)
        hasher.update(service.as_bytes());
        hasher.update(self.provider_id.0);
        hasher.update(self.zk_id.0);
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
        buff.extend_from_slice(self.zk_id.0.as_ref());
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
    Declare(DeclarationMessage),
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
