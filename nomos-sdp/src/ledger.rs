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
    ProviderNotFound(ProviderId),
    #[error("Declaration not found: {0:?}")]
    DeclarationNotFound(DeclarationId),
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
            Err(DeclarationRepositoryError::ProviderNotFound(_)) => {
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

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;

    use super::*;
    use crate::*;

    #[derive(Default, Clone)]
    struct MockDeclarationRepository {
        providers: Arc<Mutex<HashMap<ProviderId, ProviderInfo>>>,
        declarations: Arc<Mutex<HashMap<DeclarationId, Declaration>>>,
    }

    #[async_trait]
    impl DeclarationRepository for MockDeclarationRepository {
        async fn get_provider_info(
            &self,
            id: ProviderId,
        ) -> Result<ProviderInfo, DeclarationRepositoryError> {
            self.providers
                .lock()
                .unwrap()
                .get(&id)
                .copied()
                .ok_or(DeclarationRepositoryError::ProviderNotFound(id))
        }

        async fn get_declaration(
            &self,
            id: DeclarationId,
        ) -> Result<Declaration, DeclarationRepositoryError> {
            self.declarations
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .ok_or(DeclarationRepositoryError::DeclarationNotFound(id))
        }
    }

    #[derive(Default, Clone)]
    struct MockRewardsRequestSender {
        requested_rewards: Arc<Mutex<Vec<RewardMessage>>>,
    }

    #[async_trait]
    impl RewardRequestSender for MockRewardsRequestSender {
        async fn request_reward(
            &self,
            _reward_contract: MockContractAddress,
            reward_message: RewardMessage,
        ) -> Result<(), RewardsSenderError> {
            self.requested_rewards.lock().unwrap().push(reward_message);
            Ok(())
        }
    }

    #[derive(Default, Clone)]
    struct MockServiceRepository {
        service_params: Arc<Mutex<HashMap<ServiceType, ServiceParameters>>>,
    }

    #[async_trait]
    impl ServiceRepository for MockServiceRepository {
        async fn get_parameters(
            &self,
            service_type: ServiceType,
        ) -> Result<ServiceParameters, ServiceRepositoryError> {
            self.service_params
                .lock()
                .unwrap()
                .get(&service_type)
                .cloned()
                .ok_or(ServiceRepositoryError::NotFound(service_type))
        }
    }

    fn create_test_provider_key() -> ProviderId {
        ed25519_dalek::VerifyingKey::from_bytes(&[0u8; 32]).unwrap()
    }

    fn create_default_service_params() -> ServiceParameters {
        ServiceParameters {
            lock_period: 10,
            inactivity_period: 20,
            retention_period: 30,
            reward_contract: [0u8; 32],
            timestamp: 0,
        }
    }

    fn setup_ledger() -> (
        SdpLedger<MockDeclarationRepository, MockRewardsRequestSender, MockServiceRepository>,
        MockDeclarationRepository,
        MockRewardsRequestSender,
        MockServiceRepository,
    ) {
        let declaration_repo = MockDeclarationRepository::default();
        let rewards_sender = MockRewardsRequestSender::default();
        let service_repo = MockServiceRepository::default();

        {
            let mut params = service_repo.service_params.lock().unwrap();
            params.insert(ServiceType::BlendNetwork, create_default_service_params());
        };

        let ledger = SdpLedger {
            provider_repo: declaration_repo.clone(),
            reward_request_sender: rewards_sender.clone(),
            services_repo: service_repo.clone(),
            pending_block_states: HashMap::new(),
            pending_block_declarations: HashMap::new(),
            pending_rewards: HashMap::new(),
        };

        (ledger, declaration_repo, rewards_sender, service_repo)
    }

    #[tokio::test]
    async fn test_process_declare_message() {
        let (mut ledger, _, _, _) = setup_ledger();

        let provider_id = create_test_provider_key();
        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            proof_of_funds: [0u8; 64],
            provider_id,
        };

        let result = ledger
            .process_sdp_message(
                100,
                SdpMessage::Declare(declaration_message.clone()),
                [0u8; 32],
            )
            .await;

        assert!(result.is_ok());

        assert_eq!(ledger.pending_block_states.len(), 1);
        assert!(ledger.pending_block_states.contains_key(&100));

        assert_eq!(ledger.pending_block_declarations.len(), 1);
        let pending_declarations = ledger.pending_block_declarations.get(&100).unwrap();
        assert_eq!(pending_declarations.declarations.len(), 1);
    }

    #[tokio::test]
    async fn test_process_reward_message() {
        let (mut ledger, _, rewards_sender, _) = setup_ledger();
        let provider_id = create_test_provider_key();
        let declaration_id = [1u8; 32];

        let reward_message = RewardMessage {
            declaration_id,
            service_type: ServiceType::BlendNetwork,
            provider_id,
            nonce: 1,
            metadata: None,
        };

        let result = ledger
            .process_sdp_message(100, SdpMessage::Reward(reward_message.clone()), [0u8; 32])
            .await;

        assert!(result.is_ok());

        assert_eq!(ledger.pending_block_states.len(), 1);
        assert!(ledger.pending_block_states.contains_key(&100));

        let requested_rewards = rewards_sender.requested_rewards.lock().unwrap();
        assert_eq!(requested_rewards.len(), 1);
        assert_eq!(requested_rewards[0].provider_id, provider_id);
        drop(requested_rewards); // clippy strict >:)
    }

    #[tokio::test]
    async fn test_process_withdraw_message() {
        let (mut ledger, declaration_repo, rewards_sender, _) = setup_ledger();
        let provider_id = create_test_provider_key();
        let declaration_id = [1u8; 32];

        {
            let mut providers = declaration_repo.providers.lock().unwrap();
            providers.insert(
                provider_id,
                ProviderInfo::new(provider_id, declaration_id, 50),
            );
        };

        let withdraw_message = WithdrawMessage {
            declaration_id,
            service_type: ServiceType::BlendNetwork,
            provider_id,
            nonce: 1,
            metadata: Some([2u8; 256]),
        };

        let result = ledger
            .process_sdp_message(
                100,
                SdpMessage::Withdraw(withdraw_message.clone()),
                [0u8; 32],
            )
            .await;

        assert!(result.is_ok());

        assert_eq!(ledger.pending_block_states.len(), 1);
        assert!(ledger.pending_block_states.contains_key(&100));

        let requested_rewards = rewards_sender.requested_rewards.lock().unwrap();
        assert_eq!(requested_rewards.len(), 1);
        assert_eq!(requested_rewards[0].provider_id, provider_id);
        drop(requested_rewards);
    }

    #[tokio::test]
    async fn test_duplicate_declaration_prevention() {
        let (mut ledger, _, _, _) = setup_ledger();

        let provider_id = create_test_provider_key();
        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            proof_of_funds: [0u8; 64],
            provider_id,
        };

        let result1 = ledger
            .process_sdp_message(
                100,
                SdpMessage::Declare(declaration_message.clone()),
                [0u8; 32],
            )
            .await;
        assert!(result1.is_ok());

        let result2 = ledger
            .process_sdp_message(
                100,
                SdpMessage::Declare(declaration_message.clone()),
                [0u8; 32],
            )
            .await;
        assert!(result2.is_err());
    }
}
