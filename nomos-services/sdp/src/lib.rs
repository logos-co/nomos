pub mod adapters;
pub mod backends;

use std::fmt::{Debug, Display};

use adapters::{
    declaration::SdpDeclarationAdapter, rewards::SdpRewardsAdapter, services::SdpServicesAdapter,
    stakes::SdpStakesVerifierAdapter,
};
use async_trait::async_trait;
use backends::{SdpBackend, SdpBackendError};
use futures::StreamExt as _;
use nomos_sdp_core::ledger;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceStateHandle,
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
        result_sender: oneshot::Sender<Result<(), SdpBackendError>>,
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
    RewardsAdapter: ledger::ActivityContract<ContractAddress = ContractAddress, Metadata = Metadata>
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
        let mut lifecycle_stream = self.service_state.lifecycle_handle.message_stream();
        loop {
            tokio::select! {
                Some(msg) = self.service_state.inbound_relay.recv()  => {
                    self.handle_sdp_message(msg).await;
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
    async fn handle_sdp_message(&mut self, msg: SdpMessage<B>) {
        match msg {
            SdpMessage::Process {
                block_number,
                message,
            } => {
                if let Err(e) = self
                    .backend
                    .process_sdp_message(block_number, message)
                    .await
                {
                    tracing::error!("Error processing SDP message: {:?}", e);
                }
            }
            SdpMessage::MarkInBlock {
                block_number,
                result_sender,
            } => {
                let result = self.backend.mark_in_block(block_number).await;
                let result = result_sender.send(result);
                if let Err(e) = result {
                    tracing::error!("Error sending result: {:?}", e);
                }
            }
            SdpMessage::DiscardBlock(block_number) => {
                self.backend.discard_block(block_number);
            }
        }
    }
}
