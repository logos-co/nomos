use std::{collections::BTreeSet, fmt::Debug};

use nomos_core::{
    block::Block,
    da,
    mantle::{AuthenticatedMantleTx, Op},
};
use nomos_da_sampling::DaSamplingServiceMsg;
use overwatch::DynError;
use tokio::sync::oneshot;
use tracing::debug;

use crate::{LOG_TARGET, SamplingRelay};

/// Blob validation strategy.
#[async_trait::async_trait]
pub trait BlobValidation<BlobId, Tx> {
    /// Validates all the blobs in the given block.
    async fn validate(&self, block: &Block<Tx>) -> Result<(), Error>;
}

/// Checks if blobs have been sampled/validated by the sampling service
/// recently.
pub struct RecentBlobValidation<BlobId> {
    sampling_relay: SamplingRelay<BlobId>,
}

impl<BlobId> RecentBlobValidation<BlobId> {
    pub const fn new(sampling_relay: SamplingRelay<BlobId>) -> Self {
        Self { sampling_relay }
    }
}

#[async_trait::async_trait]
impl<Tx> BlobValidation<da::BlobId, Tx> for RecentBlobValidation<da::BlobId>
where
    Tx: AuthenticatedMantleTx + Sync,
{
    async fn validate(&self, block: &Block<Tx>) -> Result<(), Error> {
        debug!(target = LOG_TARGET, "Validating recent blobs");
        let sampled_blobs = get_sampled_blobs(&self.sampling_relay)
            .await
            .map_err(|_| Error::RelayError)?;
        let all_blobs_sampled = block
            .transactions()
            .flat_map(|tx| tx.mantle_tx().ops.iter())
            .filter_map(|op| {
                if let Op::ChannelBlob(op) = op {
                    Some(op.blob)
                } else {
                    None
                }
            })
            .all(|blob| sampled_blobs.contains(&blob));
        if all_blobs_sampled {
            Ok(())
        } else {
            Err(Error::InvalidBlobs)
        }
    }
}

/// Skips blob validation.
pub struct SkipBlobValidation;

#[async_trait::async_trait]
impl<BlobId, Tx> BlobValidation<BlobId, Tx> for SkipBlobValidation {
    async fn validate(&self, _: &Block<Tx>) -> Result<(), Error> {
        debug!(target = LOG_TARGET, "Skipping blob validation");
        Ok(())
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("Block contains invalid blobs")]
    InvalidBlobs,
    #[error("Relay error")]
    RelayError,
}

/// Retrieves all the blobs that have been sampled by the sampling service.
pub async fn get_sampled_blobs<BlobId>(
    sampling_relay: &SamplingRelay<BlobId>,
) -> Result<BTreeSet<BlobId>, DynError>
where
    BlobId: Send,
{
    let (sender, receiver) = oneshot::channel();
    sampling_relay
        .send(DaSamplingServiceMsg::GetValidatedBlobs {
            reply_channel: sender,
        })
        .await
        .map_err(|(e, _)| Box::new(e) as DynError)?;
    receiver.await.map_err(|e| Box::new(e) as DynError)
}
