pub mod ledger;
mod state;

use std::{collections::BTreeSet, hash::Hash};

use blake2::{Blake2b, Digest as _};
use multiaddr::Multiaddr;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub type StakeThreshold = u64;
pub type BlockNumber = u64;

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

pub type Nonce = [u8; 16];

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
pub struct RewardAddress(pub [u8; 32]);

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct DeclarationInfo {
    pub id: DeclarationId,
    pub provider_id: ProviderId,
    pub service: ServiceType,
    pub locators: Vec<Locator>,
    pub reward_address: RewardAddress,
    pub created: BlockNumber,
    pub active: Option<BlockNumber>,
    pub withdrawn: Option<BlockNumber>,
}

impl DeclarationInfo {
    #[must_use]
    pub fn new(created: BlockNumber, msg: DeclarationMessage) -> Self {
        Self {
            id: msg.declaration_id(),
            provider_id: msg.provider_id,
            service: msg.service_type,
            locators: msg.locators,
            reward_address: msg.reward_address,
            created,
            active: None,
            withdrawn: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeclarationState {
    Active,
    Inactive,
    Withdrawn,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct DeclarationMessage {
    pub service_type: ServiceType,
    pub locators: Vec<Locator>,
    pub provider_id: ProviderId,
    pub reward_address: RewardAddress,
}

impl DeclarationMessage {
    fn declaration_id(&self) -> DeclarationId {
        let mut hasher = Blake2b::new();
        let service = match self.service_type {
            ServiceType::BlendNetwork => "BN",
            ServiceType::DataAvailability => "DA",
            ServiceType::ExecutorNetwork => "EX",
        };

        hasher.update(service.as_bytes());
        hasher.update(self.provider_id.0);
        for locator in &self.locators {
            hasher.update(locator.0.as_ref());
        }
        hasher.update(self.reward_address.0);

        DeclarationId(hasher.finalize().into())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct WithdrawMessage {
    pub declaration_id: DeclarationId,
    pub service_type: ServiceType,
    pub provider_id: ProviderId,
    pub nonce: Nonce,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ActiveMessage<Metadata> {
    pub declaration_id: DeclarationId,
    pub service_type: ServiceType,
    pub provider_id: ProviderId,
    pub nonce: Nonce,
    pub metadata: Option<Metadata>,
}

impl<Metadata> ActiveMessage<Metadata> {
    pub fn activity_id(&self) -> ActivityId {
        let mut hasher = Blake2b::new();
        hasher.update(self.declaration_id.0);
        hasher.update(self.provider_id.0);
        hasher.update(self.nonce);
        ActivityId(hasher.finalize().into())
    }
}

#[derive(Copy, Clone, Debug)]
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
    pub const fn provider_id(&self) -> ProviderId {
        match self {
            Self::Declare(message) => message.provider_id,
            Self::Activity(message) => message.provider_id,
            Self::Withdraw(message) => message.provider_id,
        }
    }

    #[must_use]
    pub fn declaration_id(&self) -> DeclarationId {
        match self {
            Self::Declare(message) => message.declaration_id(),
            Self::Activity(message) => message.declaration_id,
            Self::Withdraw(message) => message.declaration_id,
        }
    }

    #[must_use]
    pub const fn service_type(&self) -> ServiceType {
        match self {
            Self::Declare(message) => message.service_type,
            Self::Activity(message) => message.service_type,
            Self::Withdraw(message) => message.service_type,
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
    pub state: DeclarationState,
    pub locators: BTreeSet<Locator>,
}
