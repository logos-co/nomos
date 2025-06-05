use std::{collections::HashMap, error::Error, fmt::Debug, marker::PhantomData};

use async_trait::async_trait;
use nomos_core::block::BlockNumber;

use crate::{
    ActiveMessage, DeclarationId, DeclarationInfo, DeclarationMessage, EventType, Nonce,
    ProviderId, SdpMessage, ServiceParameters, ServiceType, WithdrawMessage,
    state::{ProviderStateError, TransientDeclarationState},
};

#[derive(thiserror::Error, Debug)]
pub enum DeclarationsRepositoryError {
    #[error("Declaration not found: {0:?}")]
    DeclarationNotFound(DeclarationId),
    #[error("Duplicate nonce")]
    DuplicateNonce,
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

#[async_trait]
pub trait DeclarationsRepository {
    async fn get(&self, id: DeclarationId) -> Result<DeclarationInfo, DeclarationsRepositoryError>;
    async fn update(&self, declaration: DeclarationInfo)
    -> Result<(), DeclarationsRepositoryError>;
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
    async fn get_parameters(
        &self,
        service_type: ServiceType,
    ) -> Result<ServiceParameters, ServicesRepositoryError>;
}

#[derive(thiserror::Error, Debug)]
pub enum SdpLedgerError {
    #[error(transparent)]
    ProviderState(#[from] ProviderStateError),
    #[error(transparent)]
    DeclarationsRepository(#[from] DeclarationsRepositoryError),
    #[error(transparent)]
    ServicesRepository(#[from] ServicesRepositoryError),
    #[error("Provider service is already declared in declaration")]
    DuplicateServiceDeclaration,
    #[error("Declaration is not for {0:?} service")]
    IncorrectServiceType(ServiceType),
    #[error("Duplicate declaration for provider it in block")]
    DuplicateDeclarationInBlock,
    #[error("Provider declaration id and message declaration id does not match")]
    WrongDeclarationId,
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

pub struct SdpLedger<Declarations, Services, Metadata>
where
    Declarations: DeclarationsRepository,
    Services: ServicesRepository,
{
    declaration_repo: Declarations,
    services_repo: Services,
    pending_declarations: HashMap<BlockNumber, HashMap<DeclarationId, DeclarationInfo>>,
    _phantom: PhantomData<Metadata>,
}

impl<Declarations, Services, Metadata> SdpLedger<Declarations, Services, Metadata>
where
    Declarations: DeclarationsRepository + Send + Sync,
    Services: ServicesRepository + Send + Sync,
    Metadata: Send + Sync,
{
    pub fn new(declaration_repo: Declarations, services_repo: Services) -> Self {
        Self {
            declaration_repo,
            services_repo,
            pending_declarations: HashMap::new(),
            _phantom: PhantomData,
        }
    }

    fn process_declare(
        &mut self,
        block_number: BlockNumber,
        current_state: TransientDeclarationState,
        declaration_message: DeclarationMessage,
    ) -> Result<TransientDeclarationState, SdpLedgerError> {
        // Check if state can transition before inserting declaration into pending list.
        let pending_state = current_state.try_into_active(block_number, EventType::Declaration)?;
        let declaration_id = declaration_message.declaration_id();

        // Block could contain multiple transactions for the same declaration, SDP
        // considers this as a malformed block because of duplicate information.
        if self
            .pending_declarations
            .get(&block_number)
            .is_some_and(|pending| pending.contains_key(&declaration_id))
        {
            return Err(SdpLedgerError::DuplicateDeclarationInBlock);
        }

        let declaration_info = DeclarationInfo::new(block_number, declaration_message);
        let entry = self.pending_declarations.entry(block_number).or_default();
        entry.insert(declaration_id, declaration_info);

        Ok(pending_state)
    }

    async fn process_active(
        &self,
        block_number: BlockNumber,
        current_state: TransientDeclarationState,
        active_message: ActiveMessage<Metadata>,
    ) -> Result<TransientDeclarationState, SdpLedgerError> {
        // Check if state can transition before marking as active.
        let pending_state = current_state.try_into_active(block_number, EventType::Activity)?;
        let provider_id = active_message.provider_id;
        let declaration_id = active_message.declaration_id;
        let service_type = active_message.service_type;

        self.declaration_repo
            .check_nonce(provider_id, active_message.nonce)
            .await?;

        // One declaration is for one service only, allow marking active only for the
        // service that provider is providing.
        if let Ok(declaration) = self.declaration_repo.get(declaration_id).await {
            if declaration.service != service_type {
                return Err(SdpLedgerError::IncorrectServiceType(service_type));
            }
        }

        Ok(pending_state)
    }

    async fn process_withdraw(
        &self,
        block_number: BlockNumber,
        current_state: TransientDeclarationState,
        withdraw_message: WithdrawMessage,
        service_params: ServiceParameters,
    ) -> Result<TransientDeclarationState, SdpLedgerError> {
        let pending_state = current_state.try_into_withdrawn(
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

        // One declaration is for one service only, allow marking withdrawn only for the
        // service that provider is providing.
        if let Ok(declaration) = self.declaration_repo.get(declaration_id).await {
            if declaration.service != service_type {
                return Err(SdpLedgerError::IncorrectServiceType(service_type));
            }
        }

        Ok(pending_state)
    }

    pub async fn process_sdp_message(
        &mut self,
        block_number: BlockNumber,
        message: SdpMessage<Metadata>,
    ) -> Result<(), SdpLedgerError> {
        let declaration_id = message.declaration_id();
        let service_type = message.service_type();

        let maybe_pending_state = self
            .pending_declarations
            .get(&block_number)
            .and_then(|states| states.get(&declaration_id));
        let maybe_declaration_info = self.declaration_repo.get(declaration_id).await;
        let service_params = self.services_repo.get_parameters(service_type).await?;

        let current_state = if let Some(declaration_info) = maybe_pending_state {
            TransientDeclarationState::try_from_info(
                block_number,
                declaration_info.clone(),
                &service_params,
            )?
        } else {
            match (maybe_declaration_info, &message) {
                (Ok(declaration_info), _) => TransientDeclarationState::try_from_info(
                    block_number,
                    declaration_info,
                    &service_params,
                )?,
                (
                    Err(DeclarationsRepositoryError::DeclarationNotFound(_)),
                    SdpMessage::Declare(message),
                ) => {
                    let declaration_info = DeclarationInfo::new(block_number, message.clone());
                    TransientDeclarationState::try_from_info(
                        block_number,
                        declaration_info,
                        &service_params,
                    )?
                }
                (Err(err), _) => return Err(SdpLedgerError::DeclarationsRepository(err)),
            }
        };

        if current_state.declaration_id() != declaration_id {
            return Err(SdpLedgerError::WrongDeclarationId);
        }

        let pending_state = match message {
            SdpMessage::Declare(declaration_message) => {
                self.process_declare(block_number, current_state, declaration_message)?
            }
            SdpMessage::Activity(active_message) => {
                self.process_active(block_number, current_state, active_message)
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

        self.pending_declarations
            .entry(block_number)
            .or_default()
            .insert(declaration_id, pending_state.into());

        Ok(())
    }

    async fn mark_declaration_in_block(
        &mut self,
        block_number: BlockNumber,
        declaration_info: &DeclarationInfo,
    ) -> Result<(), SdpLedgerError> {
        let declaration_id = declaration_info.id;

        if let Err(err) = self.declaration_repo.update(declaration_info.clone()).await {
            // If declaration update failed - discard.
            self.pending_declarations
                .get_mut(&block_number)
                .and_then(|updates| updates.remove(&declaration_id));
            return Err(err.into());
        }

        // One provider id can declare only one service in one declaration.
        if let Some(declaration_info) = self
            .pending_declarations
            .get_mut(&block_number)
            .and_then(|updates| updates.remove(&declaration_id))
        {
            if let Err(err) = self.declaration_repo.update(declaration_info).await {
                tracing::error!("Declaration could not be updated: {err}");
            }
        }

        Ok(())
    }

    pub async fn mark_in_block(&mut self, block_number: BlockNumber) -> Result<(), SdpLedgerError> {
        let Some(updates) = self.pending_declarations.remove(&block_number) else {
            return Ok(());
        };

        for info in updates.values() {
            if let Err(err) = self.mark_declaration_in_block(block_number, info).await {
                tracing::error!("Provider information couldn't be updated: {err}");
            }
        }

        Ok(())
    }

    pub fn discard_block(&mut self, block_number: BlockNumber) {
        self.pending_declarations.remove(&block_number);
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

    type MockMetadata = [u8; 32];
    type MockSdpLedger =
        SdpLedger<MockDeclarationsRepository, MockServicesRepository, MockMetadata>;

    #[derive(Default)]
    pub struct MockBlock {
        pub messages: Vec<(SdpMessage<MockMetadata>, bool)>,
    }

    impl MockBlock {
        pub fn add_declaration(
            &mut self,
            provider_id: ProviderId,
            reward_address: RewardAddress,
            service_type: ServiceType,
            locators: Vec<Locator>,
            should_pass: bool,
        ) {
            self.messages.push((
                SdpMessage::Declare(DeclarationMessage {
                    service_type,
                    locators,
                    provider_id,
                    reward_address,
                }),
                should_pass,
            ));
        }

        pub fn add_activity(
            &mut self,
            provider_id: ProviderId,
            declaration_id: DeclarationId,
            service_type: ServiceType,
            should_pass: bool,
        ) {
            self.messages.push((
                SdpMessage::Activity(ActiveMessage {
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
            should_pass: bool,
        ) {
            self.messages.push((
                SdpMessage::Withdraw(WithdrawMessage {
                    declaration_id,
                    service_type,
                    provider_id,
                    nonce: [0u8; 16],
                }),
                should_pass,
            ));
        }
    }

    // Block Operation, short for better formatting.
    enum BOp {
        Dec(ProviderId, RewardAddress, ServiceType, Vec<Locator>),
        Act(ProviderId, DeclarationId, ServiceType),
        Wit(ProviderId, DeclarationId, ServiceType),
    }

    impl BOp {
        fn declaration_id(&self) -> DeclarationId {
            match self {
                Self::Dec(provider_id, reward_address, service_type, locators) => {
                    DeclarationMessage {
                        service_type: *service_type,
                        locators: locators.clone(),
                        provider_id: *provider_id,
                        reward_address: *reward_address,
                    }
                    .declaration_id()
                }
                Self::Act(_, _, _) => panic!(),
                Self::Wit(_, _, _) => panic!(),
            }
        }
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
                        BOp::Dec(pid, reward_addr, service, locators) => {
                            block.add_declaration(pid, reward_addr, service, locators, should_pass);
                        }
                        BOp::Act(pid, did, service) => {
                            block.add_activity(pid, did, service, should_pass);
                        }
                        BOp::Wit(pid, did, service) => {
                            block.add_withdraw(pid, did, service, should_pass);
                        }
                    }
                }
                (timestamp, block)
            })
            .collect()
    }

    #[derive(Default, Clone)]
    struct MockDeclarationsRepository {
        declarations: Arc<Mutex<HashMap<DeclarationId, DeclarationInfo>>>,
    }

    impl MockDeclarationsRepository {
        fn dump_declarations(&self) -> HashMap<DeclarationId, DeclarationInfo> {
            let d = self.declarations.lock().unwrap();
            d.clone()
        }
    }

    #[async_trait]
    impl DeclarationsRepository for MockDeclarationsRepository {
        async fn get(
            &self,
            id: DeclarationId,
        ) -> Result<DeclarationInfo, DeclarationsRepositoryError> {
            self.declarations
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .ok_or(DeclarationsRepositoryError::DeclarationNotFound(id))
        }

        async fn update(
            &self,
            declaration_info: DeclarationInfo,
        ) -> Result<(), DeclarationsRepositoryError> {
            self.declarations
                .lock()
                .unwrap()
                .insert(declaration_info.id, declaration_info);
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
    struct MockServicesRepository {
        service_params: Arc<Mutex<HashMap<ServiceType, ServiceParameters>>>,
    }

    #[async_trait]
    impl ServicesRepository for MockServicesRepository {
        async fn get_parameters(
            &self,
            service_type: ServiceType,
        ) -> Result<ServiceParameters, ServicesRepositoryError> {
            self.service_params
                .lock()
                .unwrap()
                .get(&service_type)
                .cloned()
                .ok_or(ServicesRepositoryError::NotFound(service_type))
        }
    }

    const fn default_service_params() -> ServiceParameters {
        ServiceParameters {
            lock_period: 10,
            inactivity_period: 20,
            retention_period: 30,
            timestamp: 0,
        }
    }

    fn setup_ledger() -> (
        MockSdpLedger,
        MockDeclarationsRepository,
        MockServicesRepository,
    ) {
        let declaration_repo = MockDeclarationsRepository::default();
        let service_repo = MockServicesRepository::default();

        {
            let mut params = service_repo.service_params.lock().unwrap();
            params.insert(ServiceType::BlendNetwork, default_service_params());
            params.insert(ServiceType::DataAvailability, default_service_params());
        };

        let ledger = SdpLedger {
            declaration_repo: declaration_repo.clone(),
            services_repo: service_repo.clone(),
            pending_declarations: HashMap::new(),
            _phantom: PhantomData,
        };

        (ledger, declaration_repo, service_repo)
    }

    #[tokio::test]
    async fn test_process_declare_message() {
        let (mut ledger, _, _) = setup_ledger();
        let provider_id = ProviderId([0u8; 32]);
        let reward_address = RewardAddress([0u8; 32]);
        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            provider_id,
            reward_address,
        };

        let result = ledger
            .process_sdp_message(100, SdpMessage::Declare(declaration_message.clone()))
            .await;

        assert!(result.is_ok());

        assert_eq!(ledger.pending_declarations.len(), 1);
        let pending_declarations = ledger.pending_declarations.get(&100).unwrap();
        assert_eq!(pending_declarations.len(), 1);
    }

    #[tokio::test]
    async fn test_process_activity_message() {
        let (mut ledger, declaration_repo, _) = setup_ledger();
        let provider_id = ProviderId([0u8; 32]);
        let reward_address = RewardAddress([0u8; 32]);
        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            provider_id,
            reward_address,
        };
        let declaration_id = declaration_message.declaration_id();

        {
            let mut declarations = declaration_repo.declarations.lock().unwrap();
            declarations.insert(
                declaration_id,
                DeclarationInfo::new(50, declaration_message),
            );
        };

        let active_message = ActiveMessage {
            declaration_id,
            service_type: ServiceType::BlendNetwork,
            provider_id,
            nonce: [0; 16],
            metadata: None,
        };

        let result = ledger
            .process_sdp_message(100, SdpMessage::Activity(active_message.clone()))
            .await;

        assert!(result.is_ok());

        assert_eq!(ledger.pending_declarations.len(), 1);
        assert!(ledger.pending_declarations.contains_key(&100));
    }

    #[tokio::test]
    async fn test_process_withdraw_message() {
        let (mut ledger, declaration_repo, _) = setup_ledger();
        let provider_id = ProviderId([0u8; 32]);
        let reward_address = RewardAddress([0u8; 32]);

        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            provider_id,
            reward_address,
        };
        let declaration_id = declaration_message.declaration_id();

        {
            let mut declarations = declaration_repo.declarations.lock().unwrap();
            declarations.insert(
                declaration_id,
                DeclarationInfo::new(50, declaration_message),
            );
        };

        let withdraw_message = WithdrawMessage {
            declaration_id,
            service_type: ServiceType::BlendNetwork,
            provider_id,
            nonce: [0; 16],
        };

        let result = ledger
            .process_sdp_message(100, SdpMessage::Withdraw(withdraw_message.clone()))
            .await;

        assert!(result.is_ok());

        assert_eq!(ledger.pending_declarations.len(), 1);
        assert!(ledger.pending_declarations.contains_key(&100));
    }

    #[tokio::test]
    async fn test_duplicate_declaration() {
        let (mut ledger, _, _) = setup_ledger();
        let provider_id = ProviderId([0u8; 32]);
        let reward_address = RewardAddress([0u8; 32]);
        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            provider_id,
            reward_address,
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
        let (mut ledger, declarations_repo, _) = setup_ledger();
        let pid = ProviderId([0; 32]);
        let locators = vec![Locator {
            addr: multiaddr!(Ip4([1, 2, 3, 4]), Udp(5678u16)),
        }];
        let reward_addr = RewardAddress([1; 32]);

        let declaration_a = BOp::Dec(pid, reward_addr, St::BlendNetwork, locators.clone());
        let declaration_b = BOp::Dec(pid, reward_addr, St::DataAvailability, locators.clone());
        let d1 = declaration_a.declaration_id();
        let d2 = declaration_b.declaration_id();

        let blocks = [
            (0, vec![(declaration_a, true), (declaration_b, true)]),
            (10, vec![(BOp::Act(pid, d1, St::BlendNetwork), true)]),
            (20, vec![(BOp::Act(pid, d2, St::DataAvailability), true)]),
            (
                30,
                vec![
                    (BOp::Wit(pid, d1, St::BlendNetwork), true),
                    (BOp::Wit(pid, d2, St::DataAvailability), true),
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

        let providers = declarations_repo.dump_declarations();
        assert_eq!(providers.len(), 2);

        let provider = providers.get(&d1).unwrap();
        assert_eq!(
            provider,
            &DeclarationInfo {
                provider_id: pid,
                id: d1,
                created: 0,
                active: Some(10),
                withdrawn: Some(30),
                service: ServiceType::BlendNetwork,
                locators: locators.clone(),
                reward_address: reward_addr,
            }
        );

        let provider = providers.get(&d2).unwrap();
        assert_eq!(
            provider,
            &DeclarationInfo {
                provider_id: pid,
                id: d2,
                created: 0,
                active: Some(20),
                withdrawn: Some(30),
                service: ServiceType::DataAvailability,
                locators,
                reward_address: reward_addr,
            }
        );
    }

    #[tokio::test]
    async fn test_multiple_providers_blocks() {
        let (mut ledger, declarations_repo, _) = setup_ledger();
        let p1 = ProviderId([0; 32]);
        let p2 = ProviderId([1; 32]);
        let locators = vec![Locator {
            addr: multiaddr!(Ip4([1, 2, 3, 4]), Udp(5678u16)),
        }];
        let reward_addr = RewardAddress([1; 32]);

        let declaration_a = BOp::Dec(p1, reward_addr, St::BlendNetwork, locators.clone());
        let declaration_b = BOp::Dec(p2, reward_addr, St::DataAvailability, locators.clone());
        let d1 = declaration_a.declaration_id();
        let d2 = declaration_b.declaration_id();

        let blocks = [
            (0, vec![(declaration_a, true), (declaration_b, true)]),
            (10, vec![(BOp::Act(p1, d1, St::BlendNetwork), true)]),
            (20, vec![(BOp::Act(p2, d2, St::DataAvailability), true)]),
            (
                30,
                vec![
                    // Withdrawing service that pid2 declared.
                    (BOp::Wit(p1, d2, St::BlendNetwork), false),
                    // Withdrawing service that pid1 declared.
                    (BOp::Wit(p2, d1, St::DataAvailability), false),
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

        let providers = declarations_repo.dump_declarations();
        assert_eq!(providers.len(), 2);

        let info1 = providers.get(&d1).unwrap();
        assert_eq!(
            info1,
            &DeclarationInfo {
                provider_id: p1,
                id: d1,
                created: 0,
                active: Some(10),
                withdrawn: None,
                service: ServiceType::BlendNetwork,
                locators: locators.clone(),
                reward_address: reward_addr,
            }
        );

        let info2 = providers.get(&d2).unwrap();
        assert_eq!(
            info2,
            &DeclarationInfo {
                provider_id: p2,
                id: d2,
                created: 0,
                active: Some(20),
                withdrawn: None,
                service: ServiceType::DataAvailability,
                locators,
                reward_address: reward_addr,
            }
        );
    }
}
