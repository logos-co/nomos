use std::{
    collections::{HashMap, hash_map::Entry},
    error::Error,
    fmt::Debug,
    marker::PhantomData,
};

use async_trait::async_trait;

use crate::{
    BlockNumber, Declaration, DeclarationId, DeclarationMessage, DeclarationUpdate, EventType,
    Nonce, ProviderId, ProviderInfo, RewardId, RewardMessage, SdpMessage, ServiceParameters,
    ServiceType, WithdrawMessage,
    state::{ProviderState, ProviderStateError},
};

#[derive(thiserror::Error, Debug)]
pub enum DeclarationsRepositoryError {
    #[error("Provider not found: {0:?}")]
    ProviderNotFound(ProviderId),
    #[error("Declaration not found: {0:?}")]
    DeclarationNotFound(DeclarationId),
    #[error("Duplicate nonce")]
    DuplicateNonce,
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

#[async_trait]
pub trait DeclarationsRepository {
    async fn get_provider_info(
        &self,
        provider_id: ProviderId,
    ) -> Result<ProviderInfo, DeclarationsRepositoryError>;
    async fn get_declaration(
        &self,
        declaration_id: DeclarationId,
    ) -> Result<Declaration, DeclarationsRepositoryError>;
    async fn update_provider_info(
        &self,
        provider_info: ProviderInfo,
    ) -> Result<(), DeclarationsRepositoryError>;
    async fn update_declaration(
        &self,
        declaration_update: DeclarationUpdate,
    ) -> Result<(), DeclarationsRepositoryError>;
    async fn check_nonce(
        &self,
        provider_id: ProviderId,
        nonce: Nonce,
    ) -> Result<(), DeclarationsRepositoryError>;
}

#[derive(thiserror::Error, Debug)]
pub enum ServicesRepositoryError {
    #[error("Service not found: {0:?}")]
    NotFound(ServiceType),
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

#[async_trait]
pub trait ServicesRepository {
    type ContractAddress;

    async fn get_parameters(
        &self,
        service_type: ServiceType,
    ) -> Result<ServiceParameters<Self::ContractAddress>, ServicesRepositoryError>;
}

#[derive(thiserror::Error, Debug)]
pub enum RewardsSenderError<ContractAddress> {
    #[error("Reward contract not found: {0:?}")]
    NotFound(ContractAddress),
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

#[async_trait]
pub trait RewardsRequestSender {
    type ContractAddress: Debug;
    type Metadata;

    async fn request_reward(
        &self,
        reward_contract: Self::ContractAddress,
        reward_message: RewardMessage<Self::Metadata>,
    ) -> Result<(), RewardsSenderError<Self::ContractAddress>>;
}

#[derive(thiserror::Error, Debug)]
pub enum StakesVerifierError {
    #[error("No stake")]
    NoStake,
    #[error("Stake too low")]
    StakeTooLow,
    #[error("Proof could not be verified")]
    InvalidProof,
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

#[async_trait]
pub trait StakesVerifier {
    type Proof;

    async fn verify(
        &self,
        provider_id: ProviderId,
        proof: Self::Proof,
    ) -> Result<(), StakesVerifierError>;
}

#[derive(thiserror::Error, Debug)]
pub enum SdpLedgerError<ContractAddress: Debug> {
    #[error(transparent)]
    ProviderState(#[from] ProviderStateError),
    #[error(transparent)]
    DeclarationsRepository(#[from] DeclarationsRepositoryError),
    #[error(transparent)]
    RewardsSender(#[from] RewardsSenderError<ContractAddress>),
    #[error(transparent)]
    ServicesRepository(#[from] ServicesRepositoryError),
    #[error(transparent)]
    StakesVerifier(#[from] StakesVerifierError),
    #[error("Provider service is already declared in declaration")]
    DuplicateServiceDeclaration,
    #[error("Provider does not provide {0:?} declaration")]
    ServiceNotProvided(ServiceType),
    #[error("Duplicate declaration for provider it in block")]
    DuplicateDeclarationInBlock,
    #[error("Provider declaration id and message declaration id does not match")]
    WrongDeclarationId,
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

pub struct SdpLedger<Declarations, Rewards, Services, Stakes, Proof, Metadata, ContractAddress>
where
    Declarations: DeclarationsRepository,
    Rewards: RewardsRequestSender,
    Services: ServicesRepository,
{
    declaration_repo: Declarations,
    reward_request_sender: Rewards,
    services_repo: Services,
    stake_verifier: Stakes,
    pending_providers: HashMap<BlockNumber, HashMap<ProviderId, ProviderInfo>>,
    pending_declarations: HashMap<BlockNumber, HashMap<ProviderId, DeclarationUpdate>>,
    pending_rewards: HashMap<ProviderId, RewardId>,
    _phantom: PhantomData<(Proof, Metadata, ContractAddress)>,
}

impl<Declarations, Rewards, Services, Stakes, Proof, Metadata, ContractAddress>
    SdpLedger<Declarations, Rewards, Services, Stakes, Proof, Metadata, ContractAddress>
where
    Declarations: DeclarationsRepository + Send + Sync,
    Rewards: RewardsRequestSender<Metadata = Metadata, ContractAddress = ContractAddress> + Send,
    Services: ServicesRepository<ContractAddress = ContractAddress> + Send,
    Stakes: StakesVerifier<Proof = Proof> + Send + Sync,
    ContractAddress: Debug,
{
    pub fn new(
        declaration_repo: Declarations,
        reward_request_sender: Rewards,
        services_repo: Services,
        stake_verifier: Stakes,
    ) -> Self {
        Self {
            declaration_repo,
            reward_request_sender,
            services_repo,
            stake_verifier,
            pending_providers: HashMap::new(),
            pending_declarations: HashMap::new(),
            pending_rewards: HashMap::new(),
            _phantom: PhantomData,
        }
    }

    async fn process_declare(
        &mut self,
        block_number: BlockNumber,
        current_state: ProviderState,
        declaration_message: DeclarationMessage<Proof>,
    ) -> Result<ProviderState, SdpLedgerError<ContractAddress>> {
        // Check if state can transition before inserting declaration into pending list.
        let pending_state = current_state.try_into_active(block_number, EventType::Declaration)?;
        let provider_id = declaration_message.provider_id;
        let declaration_id = declaration_message.declaration_id();
        let service_type = declaration_message.service_type;

        // One declaration (id derived from the locators set) is allowed to have
        // multiple providers. For this reason providers with a new state can declare a
        // service with an already existing declaration id.
        if let Ok(declaration) = self.declaration_repo.get_declaration(declaration_id).await {
            if declaration.has_service_provider(service_type, provider_id) {
                return Err(SdpLedgerError::DuplicateServiceDeclaration);
            }
        }

        // Multiple declarations can be included in the block, pending declarations need
        // to be checked also.
        //
        // ProviderId needs to be unique per service in declaration, the provider is
        // expected to derive the ProviderId (pubkey) using one of the service types,
        // but in ledger we just check for uniquenes of ProviderId in the declaration.
        if self
            .pending_declarations
            .get(&block_number)
            .is_some_and(|pending| pending.contains_key(&provider_id))
        {
            return Err(SdpLedgerError::DuplicateDeclarationInBlock);
        }

        let declaration_update = DeclarationUpdate::from(&declaration_message);
        let proof_of_funds = declaration_message.proof_of_funds;

        // Provider stake needs to be checked by verifying proof.
        self.stake_verifier
            .verify(provider_id, proof_of_funds)
            .await?;

        let entry = self.pending_declarations.entry(block_number).or_default();
        entry.insert(provider_id, declaration_update);

        Ok(pending_state)
    }

    async fn process_reward(
        &mut self,
        block_number: BlockNumber,
        current_state: ProviderState,
        reward_message: RewardMessage<Metadata>,
        service_params: ServiceParameters<ContractAddress>,
    ) -> Result<ProviderState, SdpLedgerError<ContractAddress>> {
        // Check if state can transition before requesting a reward.
        let pending_state = current_state.try_into_active(block_number, EventType::Reward)?;
        let provider_id = reward_message.provider_id;
        let declaration_id = reward_message.declaration_id;
        let service_type = reward_message.service_type;

        self.declaration_repo
            .check_nonce(provider_id, reward_message.nonce)
            .await?;

        // One declaration can be for multiple services, and each service could have
        // different provider id, allow reward only for the service that
        // provider is providing.
        if let Ok(declaration) = self.declaration_repo.get_declaration(declaration_id).await {
            if !declaration.has_service_provider(service_type, provider_id) {
                return Err(SdpLedgerError::ServiceNotProvided(service_type));
            }
        }

        // Reward can be sent, but state never transition, we need to allow state
        // transition if `provider_info` got rewarded, but didn't transition state for
        // some reason.
        if let Entry::Vacant(entry) = self.pending_rewards.entry(provider_id) {
            let reward_id = reward_message.reward_id();
            self.reward_request_sender
                .request_reward(service_params.reward_contract, reward_message)
                .await?;
            entry.insert(reward_id);
        }

        Ok(pending_state)
    }

    async fn process_withdraw(
        &mut self,
        block_number: BlockNumber,
        current_state: ProviderState,
        withdraw_message: WithdrawMessage<Metadata>,
        service_params: ServiceParameters<ContractAddress>,
    ) -> Result<ProviderState, SdpLedgerError<ContractAddress>> {
        let mut pending_state = current_state.try_into_withdrawn(
            block_number,
            EventType::Withdrawal,
            &service_params,
        )?;
        let provider_id = withdraw_message.provider_id;
        let declaration_id = withdraw_message.declaration_id;
        let service_type = withdraw_message.service_type;

        self.declaration_repo
            .check_nonce(provider_id, withdraw_message.nonce)
            .await?;

        // One declaration can be for multiple services, and each service could have
        // different provider id, allow withdrawals only for the service that
        // provider is providing.
        if let Ok(declaration) = self.declaration_repo.get_declaration(declaration_id).await {
            if !declaration.has_service_provider(service_type, provider_id) {
                return Err(SdpLedgerError::ServiceNotProvided(service_type));
            }
        }

        // Block can contain reward and rewardable withdraw message, only process one
        // reward for provider service per block.
        if let Entry::Vacant(entry) = self.pending_rewards.entry(provider_id) {
            if let Ok(reward_message) = RewardMessage::try_from(withdraw_message) {
                let reward_id = reward_message.reward_id();

                // If withdrawal to withdrawal with reward state transition fails, reward can't
                // be sent.
                pending_state = pending_state.try_into_withdrawn(
                    block_number,
                    EventType::Reward,
                    &service_params,
                )?;

                self.reward_request_sender
                    .request_reward(service_params.reward_contract, reward_message)
                    .await?;
                entry.insert(reward_id);
            }
        }

        Ok(pending_state)
    }

    pub async fn process_sdp_message(
        &mut self,
        block_number: BlockNumber,
        message: SdpMessage<Metadata, Proof>,
    ) -> Result<(), SdpLedgerError<ContractAddress>> {
        let provider_id = message.provider_id();
        let declaration_id = message.declaration_id();
        let service_type = message.service_type();

        let service_params = self.services_repo.get_parameters(service_type).await?;

        let maybe_pending_state = self
            .pending_providers
            .get(&block_number)
            .and_then(|states| states.get(&provider_id));

        let maybe_provider_info = self.declaration_repo.get_provider_info(provider_id).await;

        let current_state = if let Some(provider_info) = maybe_pending_state {
            ProviderState::try_from_info(block_number, provider_info, &service_params)?
        } else {
            match maybe_provider_info {
                Ok(provider_info) => {
                    ProviderState::try_from_info(block_number, &provider_info, &service_params)?
                }
                Err(DeclarationsRepositoryError::ProviderNotFound(_)) => {
                    let provider_info =
                        ProviderInfo::new(block_number, provider_id, declaration_id);
                    ProviderState::try_from_info(block_number, &provider_info, &service_params)?
                }
                Err(err) => return Err(SdpLedgerError::DeclarationsRepository(err)),
            }
        };

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

        self.pending_providers
            .entry(block_number)
            .or_default()
            .insert(provider_id, pending_state.into());

        Ok(())
    }

    async fn mark_declaration_in_block(
        &mut self,
        block_number: BlockNumber,
        provider_info: &ProviderInfo,
    ) -> Result<(), SdpLedgerError<ContractAddress>> {
        let provider_id = provider_info.provider_id;

        if let Err(err) = self
            .declaration_repo
            .update_provider_info(*provider_info)
            .await
        {
            // If provider update failed, discard declaration.
            self.pending_declarations
                .get_mut(&block_number)
                .and_then(|updates| updates.remove(&provider_id));
            return Err(err.into());
        }

        // One provider id can declare only one service in one declaration.
        if let Some(declaration_update) = self
            .pending_declarations
            .get_mut(&block_number)
            .and_then(|updates| updates.remove(&provider_id))
        {
            if let Err(err) = self
                .declaration_repo
                .update_declaration(declaration_update)
                .await
            {
                tracing::error!("Declaration could not be updated: {err}");
            }
        }

        Ok(())
    }

    pub async fn mark_in_block(
        &mut self,
        block_number: BlockNumber,
    ) -> Result<(), SdpLedgerError<ContractAddress>> {
        let Some(updates) = self.pending_providers.remove(&block_number) else {
            return Ok(());
        };

        for info in updates.values() {
            let id = info.provider_id;
            // Rewards are expected to be marked in block by entity that manages rewards.
            self.pending_rewards.remove(&id);

            if let Err(err) = self.mark_declaration_in_block(block_number, info).await {
                tracing::error!("Provider information couldn't be updated: {err}");
            }
        }

        Ok(())
    }

    pub fn discard_block(&mut self, block_number: BlockNumber) {
        self.pending_declarations.remove(&block_number);
        if let Some(updates) = self.pending_providers.remove(&block_number) {
            for ProviderInfo { provider_id, .. } in updates.values() {
                self.pending_rewards.remove(provider_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        marker::PhantomData,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use multiaddr::multiaddr;

    use super::*;
    use crate::*;

    type MockContractAddress = [u8; 32];
    type MockMetadata = [u8; 32];
    type MockProof = [u8; 32];
    type MockSdpLedger = SdpLedger<
        MockDeclarationsRepository,
        MockRewardsRequestSender,
        MockServicesRepository,
        MockStakesVerifier,
        MockProof,
        MockMetadata,
        MockContractAddress,
    >;

    #[derive(Default)]
    pub struct MockBlock {
        pub messages: Vec<(SdpMessage<MockMetadata, MockProof>, bool)>,
    }

    impl MockBlock {
        pub fn add_declaration(
            &mut self,
            provider_id: ProviderId,
            service_type: ServiceType,
            locators: Vec<Locator>,
            should_pass: bool,
        ) {
            self.messages.push((
                SdpMessage::Declare(DeclarationMessage {
                    service_type,
                    locators,
                    proof_of_funds: [0u8; 32],
                    provider_id,
                }),
                should_pass,
            ));
        }

        pub fn add_reward(
            &mut self,
            provider_id: ProviderId,
            declaration_id: DeclarationId,
            service_type: ServiceType,
            should_pass: bool,
        ) {
            self.messages.push((
                SdpMessage::Reward(RewardMessage {
                    declaration_id,
                    service_type,
                    provider_id,
                    nonce: [0u8; 16],
                    metadata: None,
                }),
                should_pass,
            ));
        }

        pub fn add_withdraw(
            &mut self,
            provider_id: ProviderId,
            declaration_id: DeclarationId,
            service_type: ServiceType,
            with_meta: bool,
            should_pass: bool,
        ) {
            self.messages.push((
                SdpMessage::Withdraw(WithdrawMessage {
                    declaration_id,
                    service_type,
                    provider_id,
                    nonce: [0u8; 16],
                    metadata: with_meta.then_some([0u8; 32]),
                }),
                should_pass,
            ));
        }
    }

    fn declaration_id(locators: &[Locator]) -> DeclarationId {
        let mut hasher = Blake2b::new();
        for locator in locators {
            hasher.update(locator.addr.as_ref());
        }
        DeclarationId(hasher.finalize().into())
    }

    // Block Operation, short for better formatting.
    enum BOp {
        Dec(ProviderId, ServiceType, Vec<Locator>),
        Rew(ProviderId, DeclarationId, ServiceType),
        Wit(ProviderId, DeclarationId, ServiceType, bool),
    }

    // Short alias for better formatting.
    type St = ServiceType;

    fn gen_blocks(blocks: Vec<(u64, Vec<(BOp, bool)>)>) -> Vec<(u64, MockBlock)> {
        blocks
            .into_iter()
            .map(|(timestamp, ops)| {
                let mut block = MockBlock::default();
                for (op, should_pass) in ops {
                    match op {
                        BOp::Dec(pid, service, locators) => {
                            block.add_declaration(pid, service, locators, should_pass);
                        }
                        BOp::Rew(pid, did, service) => {
                            block.add_reward(pid, did, service, should_pass);
                        }
                        BOp::Wit(pid, did, service, with_reward) => {
                            block.add_withdraw(pid, did, service, with_reward, should_pass);
                        }
                    }
                }
                (timestamp, block)
            })
            .collect()
    }

    #[derive(Default, Clone)]
    struct MockDeclarationsRepository {
        providers: Arc<Mutex<HashMap<ProviderId, ProviderInfo>>>,
        declarations: Arc<Mutex<HashMap<DeclarationId, Declaration>>>,
    }

    impl MockDeclarationsRepository {
        fn dump_providers(&self) -> HashMap<ProviderId, ProviderInfo> {
            let p = self.providers.lock().unwrap();
            p.clone()
        }
        fn dump_declarations(&self) -> HashMap<DeclarationId, Declaration> {
            let d = self.declarations.lock().unwrap();
            d.clone()
        }
    }

    #[async_trait]
    impl DeclarationsRepository for MockDeclarationsRepository {
        async fn get_provider_info(
            &self,
            id: ProviderId,
        ) -> Result<ProviderInfo, DeclarationsRepositoryError> {
            self.providers
                .lock()
                .unwrap()
                .get(&id)
                .copied()
                .ok_or(DeclarationsRepositoryError::ProviderNotFound(id))
        }

        async fn get_declaration(
            &self,
            id: DeclarationId,
        ) -> Result<Declaration, DeclarationsRepositoryError> {
            self.declarations
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .ok_or(DeclarationsRepositoryError::DeclarationNotFound(id))
        }

        async fn update_provider_info(
            &self,
            provider_info: ProviderInfo,
        ) -> Result<(), DeclarationsRepositoryError> {
            self.providers
                .lock()
                .unwrap()
                .insert(provider_info.provider_id, provider_info);
            Ok(())
        }

        async fn update_declaration(
            &self,
            declaration_update: DeclarationUpdate,
        ) -> Result<(), DeclarationsRepositoryError> {
            let mut declarations = self.declarations.lock().unwrap();
            if let Some(declaration) = declarations.get_mut(&declaration_update.declaration_id) {
                declaration.insert_service_provider(
                    declaration_update.provider_id,
                    declaration_update.service_type,
                );
            } else {
                let mut services = HashMap::new();
                services.insert(
                    declaration_update.service_type,
                    [declaration_update.provider_id].into(),
                );
                declarations.insert(
                    declaration_update.declaration_id,
                    Declaration {
                        declaration_id: declaration_update.declaration_id,
                        locators: declaration_update.locators,
                        services,
                    },
                );
                drop(declarations);
            }
            Ok(())
        }

        async fn check_nonce(
            &self,
            _provider_id: ProviderId,
            _nonce: Nonce,
        ) -> Result<(), DeclarationsRepositoryError> {
            Ok(())
        }
    }

    #[derive(Default, Clone)]
    struct MockRewardsRequestSender {
        requested_rewards: Arc<Mutex<Vec<RewardMessage<MockContractAddress>>>>,
    }

    #[async_trait]
    impl RewardsRequestSender for MockRewardsRequestSender {
        type ContractAddress = MockContractAddress;
        type Metadata = MockMetadata;

        async fn request_reward(
            &self,
            _reward_contract: Self::ContractAddress,
            reward_message: RewardMessage<Self::ContractAddress>,
        ) -> Result<(), RewardsSenderError<Self::ContractAddress>> {
            self.requested_rewards.lock().unwrap().push(reward_message);
            Ok(())
        }
    }

    struct MockStakesVerifier;

    #[async_trait]
    impl StakesVerifier for MockStakesVerifier {
        type Proof = MockProof;

        async fn verify(
            &self,
            _provider_id: ProviderId,
            _proof: Self::Proof,
        ) -> Result<(), StakesVerifierError> {
            Ok(())
        }
    }

    #[derive(Default, Clone)]
    struct MockServicesRepository {
        service_params: Arc<Mutex<HashMap<ServiceType, ServiceParameters<MockContractAddress>>>>,
    }

    #[async_trait]
    impl ServicesRepository for MockServicesRepository {
        type ContractAddress = MockContractAddress;

        async fn get_parameters(
            &self,
            service_type: ServiceType,
        ) -> Result<ServiceParameters<Self::ContractAddress>, ServicesRepositoryError> {
            self.service_params
                .lock()
                .unwrap()
                .get(&service_type)
                .cloned()
                .ok_or(ServicesRepositoryError::NotFound(service_type))
        }
    }

    const fn default_service_params() -> ServiceParameters<MockContractAddress> {
        ServiceParameters {
            lock_period: 10,
            inactivity_period: 20,
            retention_period: 30,
            reward_contract: [0u8; 32],
            timestamp: 0,
        }
    }

    fn setup_ledger() -> (
        MockSdpLedger,
        MockDeclarationsRepository,
        MockRewardsRequestSender,
        MockServicesRepository,
    ) {
        let declaration_repo = MockDeclarationsRepository::default();
        let rewards_sender = MockRewardsRequestSender::default();
        let service_repo = MockServicesRepository::default();
        let stake_verifier = MockStakesVerifier;

        {
            let mut params = service_repo.service_params.lock().unwrap();
            params.insert(ServiceType::BlendNetwork, default_service_params());
            params.insert(ServiceType::DataAvailability, default_service_params());
        };

        let ledger = SdpLedger {
            declaration_repo: declaration_repo.clone(),
            reward_request_sender: rewards_sender.clone(),
            services_repo: service_repo.clone(),
            stake_verifier,
            pending_providers: HashMap::new(),
            pending_declarations: HashMap::new(),
            pending_rewards: HashMap::new(),
            _phantom: PhantomData,
        };

        (ledger, declaration_repo, rewards_sender, service_repo)
    }

    #[tokio::test]
    async fn test_process_declare_message() {
        let (mut ledger, _, _, _) = setup_ledger();
        let provider_id = ProviderId([0u8; 32]);
        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            proof_of_funds: [0u8; 32],
            provider_id,
        };

        let result = ledger
            .process_sdp_message(100, SdpMessage::Declare(declaration_message.clone()))
            .await;

        assert!(result.is_ok());

        assert_eq!(ledger.pending_providers.len(), 1);
        assert!(ledger.pending_providers.contains_key(&100));

        assert_eq!(ledger.pending_declarations.len(), 1);
        let pending_declarations = ledger.pending_declarations.get(&100).unwrap();
        assert_eq!(pending_declarations.len(), 1);
    }

    #[tokio::test]
    async fn test_process_reward_message() {
        let (mut ledger, _, rewards_sender, _) = setup_ledger();
        let provider_id = ProviderId([0u8; 32]);
        let declaration_id = DeclarationId([1u8; 32]);

        let reward_message = RewardMessage {
            declaration_id,
            service_type: ServiceType::BlendNetwork,
            provider_id,
            nonce: [0; 16],
            metadata: None,
        };

        let result = ledger
            .process_sdp_message(100, SdpMessage::Reward(reward_message.clone()))
            .await;

        assert!(result.is_ok());

        assert_eq!(ledger.pending_providers.len(), 1);
        assert!(ledger.pending_providers.contains_key(&100));

        let requested_rewards = rewards_sender.requested_rewards.lock().unwrap();
        assert_eq!(requested_rewards.len(), 1);
        assert_eq!(requested_rewards[0].provider_id, provider_id);
        drop(requested_rewards); // clippy strict >:)
    }

    #[tokio::test]
    async fn test_process_withdraw_message() {
        let (mut ledger, declaration_repo, rewards_sender, _) = setup_ledger();
        let provider_id = ProviderId([0u8; 32]);
        let declaration_id = DeclarationId([1u8; 32]);

        {
            let mut providers = declaration_repo.providers.lock().unwrap();
            providers.insert(
                provider_id,
                ProviderInfo::new(50, provider_id, declaration_id),
            );
        };

        let withdraw_message = WithdrawMessage {
            declaration_id,
            service_type: ServiceType::BlendNetwork,
            provider_id,
            nonce: [0; 16],
            metadata: Some([2u8; 32]),
        };

        let result = ledger
            .process_sdp_message(100, SdpMessage::Withdraw(withdraw_message.clone()))
            .await;

        assert!(result.is_ok());

        assert_eq!(ledger.pending_providers.len(), 1);
        assert!(ledger.pending_providers.contains_key(&100));

        let requested_rewards = rewards_sender.requested_rewards.lock().unwrap();
        assert_eq!(requested_rewards.len(), 1);
        assert_eq!(requested_rewards[0].provider_id, provider_id);
        drop(requested_rewards);
    }

    #[tokio::test]
    async fn test_duplicate_declaration() {
        let (mut ledger, _, _, _) = setup_ledger();
        let provider_id = ProviderId([0u8; 32]);
        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            proof_of_funds: [0u8; 32],
            provider_id,
        };

        let result1 = ledger
            .process_sdp_message(100, SdpMessage::Declare(declaration_message.clone()))
            .await;
        assert!(result1.is_ok());

        let result2 = ledger
            .process_sdp_message(100, SdpMessage::Declare(declaration_message.clone()))
            .await;
        assert!(result2.is_err());
    }

    #[tokio::test]
    async fn test_valid_blocks() {
        let (mut ledger, declarations_repo, _, _) = setup_ledger();
        let pid = ProviderId([0; 32]);
        let locators = vec![Locator {
            addr: multiaddr!(Ip4([1, 2, 3, 4]), Udp(5678u16)),
        }];
        let did = declaration_id(&locators);
        let blocks = [
            (
                0,
                vec![
                    (BOp::Dec(pid, St::BlendNetwork, locators.clone()), true),
                    // One provider id can not be used for mutliple services in one declaration
                    (BOp::Dec(pid, St::DataAvailability, locators.clone()), false),
                ],
            ),
            (10, vec![(BOp::Rew(pid, did, St::BlendNetwork), true)]),
            // This provider is registered with the BlendNetwork service, should fail to get reward
            // for DataAvailability service.
            (20, vec![(BOp::Rew(pid, did, St::DataAvailability), false)]),
            (
                30,
                vec![
                    (BOp::Wit(pid, did, St::BlendNetwork, true), true),
                    // Provider only registered the BlendNetwork service - withdrawal for DA should
                    // fail.
                    (BOp::Wit(pid, did, St::DataAvailability, false), false),
                ],
            ),
        ]
        .into();

        let blocks = gen_blocks(blocks);
        for (block_number, block) in blocks {
            for (message, should_pass) in block.messages {
                let res = ledger.process_sdp_message(block_number, message).await;
                if should_pass {
                    assert!(res.is_ok());
                } else {
                    assert!(res.is_err());
                }
            }
            ledger.mark_in_block(block_number).await.unwrap();
        }

        let providers = declarations_repo.dump_providers();
        assert_eq!(providers.len(), 1);

        let provider = providers.get(&pid).unwrap();
        assert_eq!(
            provider,
            &ProviderInfo {
                provider_id: pid,
                declaration_id: did,
                created: 0,
                rewarded: Some(30),
                withdrawn: Some(30)
            }
        );

        let declarations = declarations_repo.dump_declarations();
        assert_eq!(declarations.len(), 1);

        let declaration = declarations.get(&did).unwrap();
        let mut expected_services = HashMap::new();
        expected_services.insert(ServiceType::BlendNetwork, [pid].into());
        assert_eq!(
            declaration,
            &Declaration {
                declaration_id: did,
                locators,
                services: expected_services,
            }
        );
    }

    #[tokio::test]
    async fn test_multiple_providers_blocks() {
        let (mut ledger, declarations_repo, _, _) = setup_ledger();
        let pid1 = ProviderId([0; 32]);
        let pid2 = ProviderId([1; 32]);
        let locators = vec![Locator {
            addr: multiaddr!(Ip4([1, 2, 3, 4]), Udp(5678u16)),
        }];
        let did = declaration_id(&locators);
        let blocks = [
            (
                0,
                vec![
                    (BOp::Dec(pid1, St::DataAvailability, locators.clone()), true),
                    (BOp::Dec(pid2, St::BlendNetwork, locators.clone()), true),
                ],
            ),
            (10, vec![(BOp::Rew(pid1, did, St::DataAvailability), true)]),
            (20, vec![(BOp::Rew(pid2, did, St::BlendNetwork), true)]),
            (
                30,
                vec![
                    // Withdrawing service that pid2 declared.
                    (BOp::Wit(pid1, did, St::BlendNetwork, true), false),
                    // Withdrawing service that pid1 declared.
                    (BOp::Wit(pid2, did, St::DataAvailability, false), false),
                ],
            ),
        ]
        .into();
        let blocks = gen_blocks(blocks);
        for (block_number, block) in blocks {
            for (message, should_pass) in block.messages {
                let res = ledger.process_sdp_message(block_number, message).await;
                if should_pass {
                    assert!(res.is_ok());
                } else {
                    assert!(res.is_err());
                }
            }
            ledger.mark_in_block(block_number).await.unwrap();
        }

        let providers = declarations_repo.dump_providers();
        assert_eq!(providers.len(), 2);

        let info1 = providers.get(&pid1).unwrap();
        assert_eq!(
            info1,
            &ProviderInfo {
                provider_id: pid1,
                declaration_id: did,
                created: 0,
                rewarded: Some(10),
                withdrawn: None, // Last transaction failed.
            }
        );

        let info2 = providers.get(&pid2).unwrap();
        assert_eq!(
            info2,
            &ProviderInfo {
                provider_id: pid2,
                declaration_id: did,
                created: 0,
                rewarded: Some(20),
                withdrawn: None,
            }
        );

        let declarations = declarations_repo.dump_declarations();
        assert_eq!(declarations.len(), 1);

        let declaration = declarations.get(&did).unwrap();
        let mut expected_services = HashMap::new();
        expected_services.insert(ServiceType::DataAvailability, [pid1].into());
        expected_services.insert(ServiceType::BlendNetwork, [pid2].into());
        assert_eq!(
            declaration,
            &Declaration {
                declaration_id: did,
                locators,
                services: expected_services,
            }
        );
    }
}
