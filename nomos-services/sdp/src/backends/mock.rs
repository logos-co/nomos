use std::{error::Error as StdError, fmt};

use async_trait::async_trait;

use super::SdpBackend;

#[derive(Debug)]
pub enum MockSdpBackendError {
    ProcessMessageFailure,
    MarkInBlockFailure,
    Other(String),
}

impl fmt::Display for MockSdpBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProcessMessageFailure => write!(f, "Process message failure"),
            Self::MarkInBlockFailure => write!(f, "Mark in block failure"),
            Self::Other(msg) => write!(f, "Other mock error: {msg}"),
        }
    }
}

impl StdError for MockSdpBackendError {}

pub struct MockSdpBackend<BN, M> {
    pub processed_messages: Vec<(BN, M)>,
    pub marked_blocks: Vec<BN>,
    pub discarded_blocks: Vec<BN>,
    pub should_fail_process: bool,
    pub should_fail_mark: bool,
}

impl<BN, M> Default for MockSdpBackend<BN, M> {
    fn default() -> Self {
        Self {
            processed_messages: Vec::new(),
            marked_blocks: Vec::new(),
            discarded_blocks: Vec::new(),
            should_fail_process: false,
            should_fail_mark: false,
        }
    }
}

impl<BN, M> MockSdpBackend<BN, M> {
    #[must_use]
    pub const fn new(should_fail_process: bool, should_fail_mark: bool) -> Self {
        Self {
            processed_messages: Vec::new(),
            marked_blocks: Vec::new(),
            discarded_blocks: Vec::new(),
            should_fail_process,
            should_fail_mark,
        }
    }
}

impl<BN, M> core::fmt::Debug for MockSdpBackend<BN, M>
where
    BN: core::fmt::Debug,
    M: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MockSdpBackend")
            .field("processed_messages", &self.processed_messages)
            .field("marked_blocks", &self.marked_blocks)
            .field("discarded_blocks", &self.discarded_blocks)
            .field("should_fail_process", &self.should_fail_process)
            .field("should_fail_mark", &self.should_fail_mark)
            .finish()
    }
}

#[async_trait]
impl<BN, M> SdpBackend for MockSdpBackend<BN, M>
where
    BN: Clone + Send + Sync + 'static,
    M: Send + Sync + 'static,
{
    type BlockNumber = BN;
    type Message = M;
    type Error = MockSdpBackendError;
    type Settings = (bool, bool); // (should_fail_process, should_fail_mark)

    fn new(settings: Self::Settings) -> Self {
        let (should_fail_process, should_fail_mark) = settings;
        Self::new(should_fail_process, should_fail_mark)
    }

    async fn process_sdp_message(
        &mut self,
        block_number: Self::BlockNumber,
        message: Self::Message,
    ) -> Result<(), Self::Error> {
        if self.should_fail_process {
            return Err(MockSdpBackendError::ProcessMessageFailure);
        }
        self.processed_messages.push((block_number, message));
        Ok(())
    }

    async fn mark_in_block(&mut self, block_number: Self::BlockNumber) -> Result<(), Self::Error> {
        if self.should_fail_mark {
            return Err(MockSdpBackendError::MarkInBlockFailure);
        }
        self.marked_blocks.push(block_number);
        Ok(())
    }

    fn discard_block(&mut self, block_number: Self::BlockNumber) {
        self.discarded_blocks.push(block_number);
    }
}

impl<BN: PartialEq, M> MockSdpBackend<BN, M> {
    pub fn was_block_processed(&self, block_number: &BN) -> bool {
        self.processed_messages
            .iter()
            .any(|(bn, _)| bn == block_number)
    }

    pub fn was_block_marked(&self, block_number: &BN) -> bool {
        self.marked_blocks.contains(block_number)
    }

    pub fn was_block_discarded(&self, block_number: &BN) -> bool {
        self.discarded_blocks.contains(block_number)
    }

    #[must_use]
    pub fn count_processed_messages(&self) -> usize {
        self.processed_messages.len()
    }
}
