use std::{
    collections::{HashMap, HashSet},
    error::Error,
};

use async_trait::async_trait;

use crate::{
    BlockNumber, Declaration, DeclarationId, DeclarationMessage, EventType, MockContractAddress,
    ProviderId, ProviderInfo, RewardId, RewardMessage, SdpMessage, SdpMessageSignature,
    ServiceParameters, ServiceType, WithdrawMessage,
    state::{ProviderState, ProviderStateError},
};

#[derive(thiserror::Error, Debug)]
pub enum DeclarationRepositoryError {
    #[error("Provider not found: {0:?}")]
    NotFound(ProviderId),
    #[error(transparent)]
    Other(Box<dyn Error + Send>),
}

#[async_trait]
pub trait DeclarationRepository {
    async fn get_provider_info(
        &self,
        id: ProviderId,
    ) -> Result<ProviderInfo, DeclarationRepositoryError>;
    async fn get_declaration(
        &self,
        id: DeclarationId,
    ) -> Result<Declaration, DeclarationRepositoryError>;
}

#[derive(thiserror::Error, Debug)]
pub enum ServiceRepositoryError {
    #[error("Service not found: {0:?}")]
    NotFound(ServiceType),
    #[error(transparent)]
    Other(Box<dyn Error + Send>),
}

#[async_trait]
pub trait ServiceRepository {
    async fn get_parameters(
        &self,
        service_type: ServiceType,
    ) -> Result<ServiceParameters, ServiceRepositoryError>;
}

#[derive(thiserror::Error, Debug)]
pub enum RewardsSenderError {
    #[error("Reward contract not found: {0:?}")]
    NotFound(MockContractAddress),
    #[error(transparent)]
    Other(Box<dyn Error + Send>),
}

#[async_trait]
pub trait RewardRequestSender {
    async fn request_reward(
        &self,
        reward_contract: MockContractAddress,
        reward_message: RewardMessage,
    ) -> Result<(), RewardsSenderError>;
}

#[derive(thiserror::Error, Debug)]
pub enum SdpLedgerError {
    #[error(transparent)]
    ProviderState(#[from] ProviderStateError),
    #[error(transparent)]
    DeclarationRepository(#[from] DeclarationRepositoryError),
    #[error(transparent)]
    RewardSender(#[from] RewardsSenderError),
    #[error(transparent)]
    ServicesRepository(#[from] ServiceRepositoryError),
    #[error("Duplicate declaration")]
    DuplicateDeclaration,
    #[error("Provider declaration id and message declaration id does not match")]
    WrongDeclarationId,
    #[error("Invalid transition")]
    InvalidProviderStateTransition,
    #[error(transparent)]
    Other(Box<dyn Error>),
}

#[derive(Default)]
struct PendingDeclarations {
    declarations: HashMap<DeclarationId, Declaration>,
}

impl PendingDeclarations {
    fn update(&mut self, message: DeclarationMessage) {
        let declaration_id = message.declaration_id();
        let DeclarationMessage {
            service_type,
            locators,
            provider_id,
            ..
        } = message;

        // A service in declaration can contain multiple providers.
        if let Some(declaration) = self.declarations.get_mut(&declaration_id) {
            declaration
                .services
                .entry(service_type)
                .or_default()
                .insert(provider_id);
        } else {
            let mut services = HashMap::new();
            services.insert(service_type, HashSet::from([provider_id]));

            let declaration = Declaration {
                declaration_id,
                locators,
                services,
            };

            self.declarations.insert(declaration_id, declaration);
        }
    }
}

pub struct SdpLedger<Declarations, Rewards, Services>
where
    Declarations: DeclarationRepository,
    Rewards: RewardRequestSender,
    Services: ServiceRepository,
{
    provider_repo: Declarations,
    reward_request_sender: Rewards,
    services_repo: Services,
    pending_block_states: HashMap<BlockNumber, HashSet<ProviderState>>,
    pending_block_declarations: HashMap<BlockNumber, PendingDeclarations>,
    pending_rewards: HashMap<ProviderId, RewardId>,
}

impl<Declarations, Rewards, Services> SdpLedger<Declarations, Rewards, Services>
where
    Declarations: DeclarationRepository + Sync + Send,
    Rewards: RewardRequestSender + Sync + Send,
    Services: ServiceRepository + Sync + Send,
{
    async fn process_declare(
        &mut self,
        block_number: BlockNumber,
        current_state: ProviderState,
        declaration_message: DeclarationMessage,
    ) -> Result<ProviderState, SdpLedgerError> {
        if !matches!(current_state, ProviderState::New(_)) {
            return Err(SdpLedgerError::DuplicateDeclaration);
        }

        // Check if state can transition before inserting declaration into pending list.
        let pending_state = current_state.to_active(block_number, EventType::Declaration)?;

        let provider_id = declaration_message.provider_id;
        let declaration_id = declaration_message.declaration_id();
        let service_type = declaration_message.service_type;

        // One declaration (id derived from the locators set) is allowed to have
        // multiple providers. For this reason providers with a new state can declare a
        // service with an already existing declaration id.
        if let Ok(declaration) = self.provider_repo.get_declaration(declaration_id).await {
            if declaration.has_service_provider(service_type, provider_id) {
                return Err(SdpLedgerError::DuplicateDeclaration);
            }
        }

        // Multiple declarations can be included in the block, pending declarations need
        // to be checked also.
        if let Some(declaration) = self
            .pending_block_declarations
            .get(&block_number)
            .and_then(|pending| pending.declarations.get(&declaration_id))
        {
            if declaration.has_service_provider(service_type, provider_id) {
                return Err(SdpLedgerError::DuplicateDeclaration);
            }
        }

        self.pending_block_declarations
            .entry(block_number)
            .or_default()
            .update(declaration_message);

        Ok(pending_state)
    }

    async fn process_reward(
        &mut self,
        block_number: BlockNumber,
        current_state: ProviderState,
        reward_message: RewardMessage,
        service_params: ServiceParameters,
    ) -> Result<ProviderState, SdpLedgerError> {
        // Check if state can transition before requesting a reward.
        let pending_state = current_state.to_active(block_number, EventType::Reward)?;
        let provider_id = reward_message.provider_id;

        // Reward can be sent, but state never transitioned, we need to allow state
        // transition if `provider_info` got rewarded, but didn't transition state for
        // some reason.
        if !self.pending_rewards.contains_key(&provider_id) {
            let reward_id = reward_message.reward_id();
            self.reward_request_sender
                .request_reward(service_params.reward_contract, reward_message)
                .await?;
            self.pending_rewards.insert(provider_id, reward_id);
        }

        Ok(pending_state)
    }

    async fn process_withdraw(
        &mut self,
        block_number: BlockNumber,
        current_state: ProviderState,
        withdraw_message: WithdrawMessage,
        service_params: ServiceParameters,
    ) -> Result<ProviderState, SdpLedgerError> {
        let mut pending_state =
            current_state.to_withdrawn(block_number, EventType::Withdrawal, &service_params)?;
        let provider_id = withdraw_message.provider_id;

        // Block can contain reward and rewardable withdraw message, only process one
        // reward for provider service per block.
        if !self.pending_rewards.contains_key(&provider_id) {
            if let Ok(reward_message) = RewardMessage::try_from(withdraw_message) {
                let reward_id = reward_message.reward_id();

                // If withdrawal to withdrawal with reward state transition fails, reward can't
                // be sent.
                pending_state =
                    pending_state.to_withdrawn(block_number, EventType::Reward, &service_params)?;

                self.reward_request_sender
                    .request_reward(service_params.reward_contract, reward_message)
                    .await?;
                self.pending_rewards.insert(provider_id, reward_id);
            }
        }

        Ok(pending_state)
    }

    pub async fn process_sdp_message(
        &mut self,
        block_number: BlockNumber,
        message: SdpMessage,
        _message_sig: SdpMessageSignature,
    ) -> Result<(), SdpLedgerError> {
        // TODO: check signature.

        let maybe_provider_info = self
            .provider_repo
            .get_provider_info(message.provider_id())
            .await;

        let provider_id = message.provider_id();
        let declaration_id = message.declaration_id();
        let service_type = message.service_type();

        let service_params = self.services_repo.get_parameters(service_type).await?;

        let current_state = match maybe_provider_info {
            Ok(provider_info) => {
                ProviderState::from_info(block_number, provider_info, &service_params)
            }
            Err(DeclarationRepositoryError::NotFound(_)) => {
                ProviderState::new(block_number, provider_id, declaration_id)
            }
            Err(err) => return Err(SdpLedgerError::DeclarationRepository(err)),
        };

        // ProviderId has to be unique per declaration.
        if current_state.declaration_id() != declaration_id {
            return Err(SdpLedgerError::WrongDeclarationId);
        }

        let pending_state = match message {
            SdpMessage::Declare(declaration_message) => {
                self.process_declare(block_number, current_state, declaration_message)
                    .await?
            }
            SdpMessage::Reward(reward_message) => {
                self.process_reward(block_number, current_state, reward_message, service_params)
                    .await?
            }
            SdpMessage::Withdraw(withdraw_message) => {
                self.process_withdraw(
                    block_number,
                    current_state,
                    withdraw_message,
                    service_params,
                )
                .await?
            }
        };

        self.pending_block_states
            .entry(block_number)
            .or_default()
            .insert(pending_state);

        Ok(())
    }
}
