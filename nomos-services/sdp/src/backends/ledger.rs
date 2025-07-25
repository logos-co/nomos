use std::{collections::HashMap, marker::PhantomData};

use nomos_core::{
    block::BlockNumber,
    sdp::{
        state::TransientDeclarationState, ActiveMessage, DeclarationId, DeclarationInfo,
        DeclarationMessage, EventType, SdpMessage, ServiceParameters, WithdrawMessage,
    },
};

use super::{
    DeclarationsRepository, DeclarationsRepositoryError, SdpBackend, SdpBackendError,
    SdpLedgerError, ServicesRepository,
};
use crate::adapters::{
    declaration::repository::SdpDeclarationAdapter,
    services::services_repository::SdpServicesAdapter,
};

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
        let declaration_id = active_message.declaration_id;

        self.declaration_repo
            .check_nonce(declaration_id, active_message.nonce)
            .await?;

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
        let declaration_id = withdraw_message.declaration_id;

        self.declaration_repo
            .check_nonce(declaration_id, withdraw_message.nonce)
            .await?;

        Ok(pending_state)
    }

    async fn get_current_state(
        &self,
        block_number: BlockNumber,
        message: &SdpMessage<Metadata>,
    ) -> Result<(TransientDeclarationState, ServiceParameters), SdpLedgerError> {
        let declaration_id = message.declaration_id();

        let maybe_pending_state = self
            .pending_declarations
            .get(&block_number)
            .and_then(|states| states.get(&declaration_id));
        let maybe_declaration_info = self.declaration_repo.get(declaration_id).await;

        let (current_state, service_params) = if let Some(declaration_info) = maybe_pending_state {
            let service_params = self
                .services_repo
                .get_parameters(declaration_info.service)
                .await?;
            (
                TransientDeclarationState::try_from_info(
                    block_number,
                    declaration_info.clone(),
                    &service_params,
                )?,
                service_params,
            )
        } else {
            match (maybe_declaration_info, &message) {
                (Ok(declaration_info), _) => {
                    let service_params = self
                        .services_repo
                        .get_parameters(declaration_info.service)
                        .await?;
                    (
                        TransientDeclarationState::try_from_info(
                            block_number,
                            declaration_info,
                            &service_params,
                        )?,
                        service_params,
                    )
                }
                (
                    Err(DeclarationsRepositoryError::DeclarationNotFound(_)),
                    SdpMessage::Declare(message),
                ) => {
                    let declaration_info = DeclarationInfo::new(block_number, message.clone());
                    let service_params = self
                        .services_repo
                        .get_parameters(declaration_info.service)
                        .await?;
                    (
                        TransientDeclarationState::try_from_info(
                            block_number,
                            declaration_info,
                            &service_params,
                        )?,
                        service_params,
                    )
                }
                (Err(err), _) => return Err(SdpLedgerError::DeclarationsRepository(err)),
            }
        };

        Ok((current_state, service_params))
    }

    pub async fn process_sdp_message(
        &mut self,
        block_number: BlockNumber,
        message: SdpMessage<Metadata>,
    ) -> Result<(), SdpLedgerError> {
        let declaration_id = message.declaration_id();
        let (current_state, service_params) =
            self.get_current_state(block_number, &message).await?;

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

#[async_trait::async_trait]
impl<Declarations, Services, Metadata> SdpBackend for SdpLedger<Declarations, Services, Metadata>
where
    Metadata: Send + Sync + 'static,
    Declarations: SdpDeclarationAdapter + Send + Sync,
    Services: SdpServicesAdapter + Send + Sync,
{
    type Message = SdpMessage<Metadata>;
    type DeclarationAdapter = Declarations;
    type ServicesAdapter = Services;

    fn init(
        declaration_adapter: Self::DeclarationAdapter,
        services_adapter: Self::ServicesAdapter,
    ) -> Self {
        Self::new(declaration_adapter, services_adapter)
    }

    async fn process_sdp_message(
        &mut self,
        block_number: BlockNumber,
        message: Self::Message,
    ) -> Result<(), SdpBackendError> {
        self.process_sdp_message(block_number, message)
            .await
            .map_err(Into::into)
    }

    async fn mark_in_block(&mut self, block_number: BlockNumber) -> Result<(), SdpBackendError> {
        self.mark_in_block(block_number).await.map_err(Into::into)
    }

    fn discard_block(&mut self, block_number: BlockNumber) {
        self.discard_block(block_number);
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
    use nomos_core::sdp::{
        ActiveMessage, DeclarationId, DeclarationInfo, DeclarationMessage, Locator, Nonce,
        ProviderId, SdpMessage, ServiceParameters, ServiceType, WithdrawMessage, ZkPublicKey,
    };

    use super::{
        DeclarationsRepository, DeclarationsRepositoryError, SdpLedger, ServicesRepository,
    };
    use crate::backends::ServicesRepositoryError;

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
            zk_id: ZkPublicKey,
            service_type: ServiceType,
            locators: Vec<Locator>,
            should_pass: bool,
        ) {
            self.messages.push((
                SdpMessage::Declare(DeclarationMessage {
                    service_type,
                    locators,
                    provider_id,
                    zk_id,
                }),
                should_pass,
            ));
        }

        pub fn add_activity(
            &mut self,
            _provider_id: ProviderId,
            declaration_id: DeclarationId,
            _service_type: ServiceType,
            should_pass: bool,
        ) {
            self.messages.push((
                SdpMessage::Activity(ActiveMessage {
                    declaration_id,
                    nonce: 1,
                    metadata: None,
                }),
                should_pass,
            ));
        }

        pub fn add_withdraw(
            &mut self,
            _provider_id: ProviderId,
            declaration_id: DeclarationId,
            _service_type: ServiceType,
            should_pass: bool,
        ) {
            self.messages.push((
                SdpMessage::Withdraw(WithdrawMessage {
                    declaration_id,
                    nonce: 2,
                }),
                should_pass,
            ));
        }
    }

    // Block Operation, short for better formatting.
    enum BOp {
        Dec(ProviderId, ZkPublicKey, ServiceType, Vec<Locator>),
        Act(ProviderId, DeclarationId, ServiceType),
        Wit(ProviderId, DeclarationId, ServiceType),
    }

    impl BOp {
        fn declaration_id(&self) -> DeclarationId {
            match self {
                Self::Dec(provider_id, zk_id, service_type, locators) => DeclarationMessage {
                    service_type: *service_type,
                    locators: locators.clone(),
                    provider_id: *provider_id,
                    zk_id: *zk_id,
                }
                .declaration_id(),
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
                        BOp::Dec(pid, zk_id, service, locators) => {
                            block.add_declaration(pid, zk_id, service, locators, should_pass);
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
            _declaration_id: DeclarationId,
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
        let zk_id = ZkPublicKey([0u8; 32]);
        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            provider_id,
            zk_id,
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
        let zk_id = ZkPublicKey([0u8; 32]);
        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            provider_id,
            zk_id,
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
            nonce: 1,
            metadata: None,
        };

        let result = ledger
            .process_sdp_message(100, SdpMessage::Activity(active_message))
            .await;

        assert!(result.is_ok());

        assert_eq!(ledger.pending_declarations.len(), 1);
        assert!(ledger.pending_declarations.contains_key(&100));
    }

    #[tokio::test]
    async fn test_process_withdraw_message() {
        let (mut ledger, declaration_repo, _) = setup_ledger();
        let provider_id = ProviderId([0u8; 32]);
        let zk_id = ZkPublicKey([0u8; 32]);

        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            provider_id,
            zk_id,
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
            nonce: 1,
        };

        let result = ledger
            .process_sdp_message(100, SdpMessage::Withdraw(withdraw_message))
            .await;

        assert!(result.is_ok());

        assert_eq!(ledger.pending_declarations.len(), 1);
        assert!(ledger.pending_declarations.contains_key(&100));
    }

    #[tokio::test]
    async fn test_duplicate_declaration() {
        let (mut ledger, _, _) = setup_ledger();
        let provider_id = ProviderId([0u8; 32]);
        let zk_id = ZkPublicKey([0u8; 32]);
        let declaration_message = DeclarationMessage {
            service_type: ServiceType::BlendNetwork,
            locators: vec![],
            provider_id,
            zk_id,
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
        let locators = vec![Locator(multiaddr!(Ip4([1, 2, 3, 4]), Udp(5678u16)))];
        let zk_id = ZkPublicKey([1; 32]);

        let declaration_a = BOp::Dec(pid, zk_id, St::BlendNetwork, locators.clone());
        let declaration_b = BOp::Dec(pid, zk_id, St::DataAvailability, locators.clone());
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
                zk_id,
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
                zk_id,
            }
        );
    }

    #[tokio::test]
    async fn test_multiple_providers_blocks() {
        let (mut ledger, declarations_repo, _) = setup_ledger();
        let p1 = ProviderId([0; 32]);
        let p2 = ProviderId([1; 32]);
        let locators = vec![Locator(multiaddr!(Ip4([1, 2, 3, 4]), Udp(5678u16)))];
        let zk_id = ZkPublicKey([1; 32]);

        let declaration_a = BOp::Dec(p1, zk_id, St::BlendNetwork, locators.clone());
        let declaration_b = BOp::Dec(p2, zk_id, St::DataAvailability, locators.clone());
        let d1 = declaration_a.declaration_id();
        let d2 = declaration_b.declaration_id();

        let blocks = [
            (0, vec![(declaration_a, true), (declaration_b, true)]),
            (10, vec![(BOp::Act(p1, d1, St::BlendNetwork), true)]),
            (20, vec![(BOp::Act(p2, d2, St::DataAvailability), true)]),
            (
                30,
                vec![
                    // Withdrawing service that pid1 declared. If different provider sends such
                    // withdrawal message - the Op signature verification should fail, but it's
                    // checked in a different layer, before passing message to the ledger.
                    (BOp::Wit(p1, d1, St::BlendNetwork), true),
                    // Withdrawing service that pid2 declared.
                    (BOp::Wit(p2, d2, St::DataAvailability), true),
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
                withdrawn: Some(30),
                service: ServiceType::BlendNetwork,
                locators: locators.clone(),
                zk_id,
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
                withdrawn: Some(30),
                service: ServiceType::DataAvailability,
                locators,
                zk_id,
            }
        );
    }
}
