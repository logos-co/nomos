pub mod adapters;
pub mod backends;

use std::fmt::{Debug, Display};

use adapters::{
    declaration::SdpDeclarationAdapter, rewards::SdpRewardsAdapter, services::SdpServicesAdapter,
    stakes::SdpStakesVerifierAdapter,
};
use async_trait::async_trait;
use backends::SdpBackend;
use futures::StreamExt;
use nomos_sdp_core::ledger;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
use services_utils::overwatch::lifecycle;
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum SdpMessage<B: SdpBackend> {
    Process {
        block_number: B::BlockNumber,
        message: B::Message,
    },

    MarkInBlock {
        block_number: B::BlockNumber,
        result_sender: oneshot::Sender<Result<(), DynError>>,
    },
    DiscardBlock(B::BlockNumber),
}

pub struct SdpService<
    B: SdpBackend + Send + Sync + 'static,
    DeclarationAdapter,
    RewardsAdapter,
    StakesVerifierAdapter,
    ServicesAdapter,
    Metadata,
    ContractAddress,
    Proof,
    RuntimeServiceId,
> where
    DeclarationAdapter: SdpDeclarationAdapter + Send + Sync,
    RewardsAdapter: SdpRewardsAdapter + Send + Sync,
    ServicesAdapter: SdpServicesAdapter + Send + Sync,
    StakesVerifierAdapter: SdpStakesVerifierAdapter + Send + Sync,
    Metadata: Send + Sync + 'static,
    Proof: Send + Sync + 'static,
    ContractAddress: Debug + Send + Sync + 'static,
{
    backend: B,
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
}

impl<
        B,
        DeclarationAdapter,
        RewardsAdapter,
        StakesVerifierAdapter,
        ServicesAdapter,
        Metadata,
        ContractAddress,
        Proof,
        RuntimeServiceId,
    > ServiceData
    for SdpService<
        B,
        DeclarationAdapter,
        RewardsAdapter,
        StakesVerifierAdapter,
        ServicesAdapter,
        Metadata,
        ContractAddress,
        Proof,
        RuntimeServiceId,
    >
where
    B: SdpBackend + Send + Sync + 'static,
    DeclarationAdapter: SdpDeclarationAdapter + Send + Sync,
    RewardsAdapter: SdpRewardsAdapter + Send + Sync,
    ServicesAdapter: SdpServicesAdapter + Send + Sync,
    StakesVerifierAdapter: SdpStakesVerifierAdapter + Send + Sync,
    Metadata: Send + Sync + 'static,
    Proof: Send + Sync + 'static,
    ContractAddress: Debug + Send + Sync + 'static,
{
    type Settings = ();
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = SdpMessage<B>;
}

#[async_trait]
impl<
        B: SdpBackend,
        DeclarationAdapter,
        RewardsAdapter,
        StakesVerifierAdapter,
        ServicesAdapter,
        Metadata,
        ContractAddress,
        Proof,
        RuntimeServiceId,
    > ServiceCore<RuntimeServiceId>
    for SdpService<
        B,
        DeclarationAdapter,
        RewardsAdapter,
        StakesVerifierAdapter,
        ServicesAdapter,
        Metadata,
        ContractAddress,
        Proof,
        RuntimeServiceId,
    >
where
    B: SdpBackend<
            DeclarationAdapter = DeclarationAdapter,
            ServicesAdapter = ServicesAdapter,
            RewardsAdapter = RewardsAdapter,
            StakesVerifierAdapter = StakesVerifierAdapter,
        > + Send
        + Sync
        + 'static,
    DeclarationAdapter: ledger::DeclarationsRepository + SdpDeclarationAdapter + Send + Sync,
    RewardsAdapter: ledger::RewardsRequestSender<ContractAddress = ContractAddress, Metadata = Metadata>
        + SdpRewardsAdapter
        + Send
        + Sync,
    ServicesAdapter: ledger::ServicesRepository<ContractAddress = ContractAddress>
        + SdpServicesAdapter
        + Send
        + Sync,
    StakesVerifierAdapter:
        ledger::StakesVerifier<Proof = Proof> + SdpStakesVerifierAdapter + Send + Sync,
    Metadata: Send + Sync + 'static,
    Proof: Send + Sync + 'static,
    ContractAddress: Debug + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + Sync + 'static,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _initstate: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        let declaration_adapter = DeclarationAdapter::new();
        let services_adapter = ServicesAdapter::new();
        let stake_verifier_adapter = StakesVerifierAdapter::new();
        let rewards_adapter = RewardsAdapter::new();
        Ok(Self {
            backend: B::init(
                declaration_adapter,
                rewards_adapter,
                services_adapter,
                stake_verifier_adapter,
            ),
            service_state,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            mut backend,
            service_state:
                OpaqueServiceStateHandle::<Self, RuntimeServiceId> {
                    mut inbound_relay,
                    lifecycle_handle,
                    ..
                },
        } = self;
        let mut lifecycle_stream = lifecycle_handle.message_stream();
        let backend = &mut backend;
        loop {
            tokio::select! {
                Some(msg) = inbound_relay.recv() => {
                    Self::handle_sdp_message(msg, backend).await;
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

impl<
        B: SdpBackend + Send + Sync + 'static,
        DeclarationAdapter: SdpDeclarationAdapter + Send + Sync,
        RewardsAdapter: SdpRewardsAdapter + Send + Sync,
        StakesVerifierAdapter: SdpStakesVerifierAdapter + Send + Sync,
        ServicesAdapter: SdpServicesAdapter + Send + Sync,
        Metadata: Send + Sync + 'static,
        ContractAddress: Debug + Send + Sync + 'static,
        Proof: Send + Sync + 'static,
        RuntimeServiceId: Send + Sync + 'static,
    >
    SdpService<
        B,
        DeclarationAdapter,
        RewardsAdapter,
        StakesVerifierAdapter,
        ServicesAdapter,
        Metadata,
        ContractAddress,
        Proof,
        RuntimeServiceId,
    >
{
    async fn handle_sdp_message(msg: SdpMessage<B>, backend: &mut B) {
        match msg {
            SdpMessage::Process {
                block_number,
                message,
            } => {
                if let Err(e) = backend.process_sdp_message(block_number, message).await {
                    tracing::error!("Error processing SDP message: {}", e);
                }
            }
            SdpMessage::MarkInBlock {
                block_number,
                result_sender,
            } => {
                let result = backend.mark_in_block(block_number).await;
                let result = result_sender.send(result);
                if let Err(e) = result {
                    tracing::error!("Error sending result: {:?}", e);
                }
            }
            SdpMessage::DiscardBlock(block_number) => {
                backend.discard_block(block_number);
            }
        }
    }
}
