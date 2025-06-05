pub mod ledger;
mod state;

use std::{collections::BTreeSet, hash::Hash};

use blake2::{Blake2b, Digest as _};
use multiaddr::Multiaddr;
use nomos_core::block::BlockNumber;

pub type StakeThreshold = u64;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Locator {
    addr: Multiaddr,
}

impl Locator {
    #[must_use]
    pub const fn new(addr: Multiaddr) -> Self {
        Self { addr }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ServiceType {
    BlendNetwork,
    DataAvailability,
    ExecutorNetwork,
}

pub type Nonce = [u8; 16];

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct ProviderId(pub [u8; 32]);

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct DeclarationId(pub [u8; 32]);

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct ActivityId(pub [u8; 32]);

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
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

#[derive(Debug, Clone)]
pub enum DeclarationState {
    Active,
    Inactive,
    Withdrawn,
}

#[derive(Clone)]
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
            hasher.update(locator.addr.as_ref());
        }
        hasher.update(self.reward_address.0);

        DeclarationId(hasher.finalize().into())
    }
}

#[derive(Clone)]
pub struct WithdrawMessage {
    pub declaration_id: DeclarationId,
    pub service_type: ServiceType,
    pub provider_id: ProviderId,
    pub nonce: Nonce,
}

#[derive(Clone)]
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

#[derive(Debug, Clone)]
pub struct FinalizedBlockEvent {
    pub block_number: BlockNumber,
    pub updates: Vec<FinalizedBlockEventUpdate>,
}

#[derive(Debug, Clone)]
pub struct FinalizedBlockEventUpdate {
    pub service_type: ServiceType,
    pub provider_id: ProviderId,
    pub state: DeclarationState,
    pub locators: BTreeSet<Locator>,
}
