use std::fmt::Debug;

use nomos_sdp_core::{
    ledger::{
        DeclarationsRepository, RewardsRequestSender, SdpLedger, SdpLedgerError,
        ServicesRepository, StakesVerifier,
    },
    BlockNumber, SdpMessage,
};

use super::SdpBackend;

#[async_trait::async_trait]
impl<Declarations, Rewards, Services, Stakes, Proof, Metadata, ContractAddress> SdpBackend
    for SdpLedger<Declarations, Rewards, Services, Stakes, Proof, Metadata, ContractAddress>
where
    Declarations: DeclarationsRepository + Send + Sync + Clone + 'static,
    Rewards: RewardsRequestSender<Metadata = Metadata, ContractAddress = ContractAddress>
        + Send
        + Clone
        + Sync
        + 'static,
    Services: ServicesRepository<ContractAddress = ContractAddress> + Send + Sync + Clone + 'static,
    Stakes: StakesVerifier<Proof = Proof> + Send + Clone + Sync + 'static,
    ContractAddress: Debug + Send + Sync + 'static,
    Proof: Send + Sync + 'static,
    Metadata: Send + Sync + 'static,
    SdpLedgerError<ContractAddress>: Send + Sync,
{
    type BlockNumber = BlockNumber;
    type Message = SdpMessage<Metadata, Proof>;
    type Error = SdpLedgerError<ContractAddress>;
    type Settings = (Declarations, Rewards, Services, Stakes);

    fn new(settings: Self::Settings) -> Self {
        let (declaration_repo, reward_request_sender, services_repo, stake_verifier) = settings;

        Self::new(
            declaration_repo,
            reward_request_sender,
            services_repo,
            stake_verifier,
        )
    }

    async fn process_sdp_message(
        &mut self,
        block_number: Self::BlockNumber,
        message: Self::Message,
    ) -> Result<(), Self::Error> {
        self.process_sdp_message(block_number, message).await
    }

    async fn mark_in_block(&mut self, block_number: Self::BlockNumber) -> Result<(), Self::Error> {
        self.mark_in_block(block_number).await
    }

    fn discard_block(&mut self, block_number: Self::BlockNumber) {
        self.discard_block(block_number);
    }
}
