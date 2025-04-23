pub mod ledger;
pub mod state;

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use blake2::{Blake2b, Digest};
use multiaddr::Multiaddr;

pub type StakeThreshold = u64;
pub type BlockNumber = u64;

pub struct MinStake {
    pub threshold: StakeThreshold,
    pub timestamp: BlockNumber,
}

#[derive(Clone, Debug)]
pub struct ServiceParameters<ContractAddress> {
    pub lock_period: u64,
    pub inactivity_period: u64,
    pub retention_period: u64,
    pub activity_contract: ContractAddress,
    pub timestamp: BlockNumber,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Locator {
    addr: Multiaddr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ServiceType {
    BlendNetwork,
    DataAvailability,
    ExecutorNetwork,
    GenericRestaking,
}

pub type Nonce = [u8; 16];

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct ProviderId(pub [u8; 32]);

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct DeclarationId(pub [u8; 32]);

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct ActivityId(pub [u8; 32]);

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct ProviderInfo {
    pub provider_id: ProviderId,
    pub declaration_id: DeclarationId,
    pub created: BlockNumber,
    pub active: Option<BlockNumber>,
    pub withdrawn: Option<BlockNumber>,
}

impl ProviderInfo {
    #[must_use]
    pub const fn new(
        block_number: BlockNumber,
        provider_id: ProviderId,
        declaration_id: DeclarationId,
    ) -> Self {
        Self {
            provider_id,
            declaration_id,
            created: block_number,
            active: None,
            withdrawn: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub declaration_id: DeclarationId,
    pub locators: Vec<Locator>,
    pub services: HashMap<ServiceType, HashSet<ProviderId>>,
}

impl Declaration {
    #[must_use]
    pub fn has_service_provider(&self, service_type: ServiceType, provider_id: ProviderId) -> bool {
        self.services
            .get(&service_type)
            .is_some_and(|service| service.contains(&provider_id))
    }

    pub fn insert_service_provider(&mut self, provider_id: ProviderId, service_type: ServiceType) {
        self.services
            .entry(service_type)
            .or_default()
            .insert(provider_id);
    }
}

#[derive(Clone, Debug)]
pub struct DeclarationUpdate {
    pub declaration_id: DeclarationId,
    pub provider_id: ProviderId,
    pub service_type: ServiceType,
    pub locators: Vec<Locator>,
}

impl<Proof> From<&DeclarationMessage<Proof>> for DeclarationUpdate {
    fn from(message: &DeclarationMessage<Proof>) -> Self {
        Self {
            declaration_id: message.declaration_id(),
            provider_id: message.provider_id,
            service_type: message.service_type,
            locators: message.locators.clone(),
        }
    }
}

#[derive(Clone)]
pub struct DeclarationMessage<Proof> {
    pub service_type: ServiceType,
    pub locators: Vec<Locator>,
    pub proof_of_funds: Proof,
    pub provider_id: ProviderId,
}

impl<Proof> DeclarationMessage<Proof> {
    fn declaration_id(&self) -> DeclarationId {
        let mut hasher = Blake2b::new();
        for locator in &self.locators {
            hasher.update(locator.addr.as_ref());
        }
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

pub enum SdpMessage<Metadata, Proof> {
    Declare(DeclarationMessage<Proof>),
    Activity(ActiveMessage<Metadata>),
    Withdraw(WithdrawMessage),
}

impl<Metadata, Proof> SdpMessage<Metadata, Proof> {
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
