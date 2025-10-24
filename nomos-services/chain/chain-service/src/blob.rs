#[cfg(test)]
use std::any::Any;
use std::{collections::BTreeSet, fmt::Debug, num::NonZero};

use cryptarchia_engine::Slot;
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

    /// A test utility to downcast the trait object.
    #[cfg(test)]
    fn as_any(&self) -> &dyn Any;
}

/// Creates a [`BlobValidation`] instance based on the [`BlockType`] and the
/// block slot.
///
/// If the block is outside the blob validation window, which is calculated
/// based on the current slot and the consensus base period length, `None` is
/// returned. Otherwise, a [`RecentBlobValidation`] or
/// [`HistoricBlobValidation`] instance is returned depending on the block type.
pub fn new_blob_validation<Tx>(
    block_slot: Slot,
    block_type: &BlockType,
    current_slot: Slot,
    consensus_base_period_length: NonZero<u64>,
    sampling_relay: &SamplingRelay<da::BlobId>,
) -> Option<Box<dyn BlobValidation<da::BlobId, Tx> + Send + Sync + 'static>>
where
    Tx: AuthenticatedMantleTx + Sync,
{
    if !should_validate_blobs(block_slot, current_slot, consensus_base_period_length) {
        return None;
    }

    match block_type {
        BlockType::Recent => Some(Box::new(RecentBlobValidation::new(sampling_relay.clone()))),
        BlockType::Historic => Some(Box::new(HistoricBlobValidation)),
    }
}

/// Block type to determine the blob validation strategy.
pub enum BlockType {
    /// Block received through recent block propagation.
    Recent,
    /// Block retrieved manually (e.g. chain bootstrapping or orphan handling).
    Historic,
}

/// Blob validation strategy that verifies whether all blobs in a block
/// have been recently sampled by the DA sampling service.
///
/// Used when a block is received through recent block propagation,
/// under the assumption that the DA sampling service has already
/// sampled and validated the blobs.
struct RecentBlobValidation<BlobId> {
    sampling_relay: SamplingRelay<BlobId>,
}

impl<BlobId> RecentBlobValidation<BlobId> {
    const fn new(sampling_relay: SamplingRelay<BlobId>) -> Self {
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

    #[cfg(test)]
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Blob validation strategy that triggers historic sampling via the DA sampling
/// service.
///
/// Used when a block is retrieved manually (e.g. during chain bootstrapping or
/// orphan handling), instead of through recent block propagation.
struct HistoricBlobValidation;

#[async_trait::async_trait]
impl<BlobId, Tx> BlobValidation<BlobId, Tx> for HistoricBlobValidation {
    async fn validate(&self, _: &Block<Tx>) -> Result<(), Error> {
        debug!(target = LOG_TARGET, "Validating historic blobs");
        // TODO: trigger historic sampling and wait for completion: https://github.com/logos-co/nomos/issues/1838
        Ok(())
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn Any {
        self
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

fn should_validate_blobs(
    block_slot: Slot,
    current_slot: Slot,
    consensus_base_period_length: NonZero<u64>,
) -> bool {
    current_slot.saturating_sub(block_slot)
        <= blob_validation_window_in_slots(consensus_base_period_length)
}

const fn blob_validation_window_in_slots(consensus_base_period_length: NonZero<u64>) -> Slot {
    Slot::new(consensus_base_period_length.get() / 2)
}

#[cfg(test)]
mod tests {
    use nomos_core::mantle::SignedMantleTx;
    use tokio::sync::mpsc;

    use super::*;

    #[test]
    fn test_blob_validation_window_in_slots() {
        assert_eq!(
            blob_validation_window_in_slots(10.try_into().unwrap()),
            5.into()
        );
        assert_eq!(
            blob_validation_window_in_slots(7.try_into().unwrap()),
            3.into() // Should round down
        );
        assert_eq!(
            blob_validation_window_in_slots(1.try_into().unwrap()),
            0.into() // Should round down
        );
    }

    #[test]
    fn test_should_validate_blobs() {
        // (103 - 100) <= (10 / 2)
        assert!(should_validate_blobs(
            100.into(),
            103.into(),
            10.try_into().unwrap()
        ));
        // (105 - 100) <= (10 / 2)
        assert!(should_validate_blobs(
            100.into(),
            105.into(),
            10.try_into().unwrap()
        ));
        // (106 - 100) > (10 / 2)
        assert!(!should_validate_blobs(
            100.into(),
            106.into(),
            10.try_into().unwrap()
        ));
        // saturating(100 - 101) <= (10 / 2)
        assert!(should_validate_blobs(
            101.into(),
            100.into(),
            10.try_into().unwrap()
        ));
    }

    #[test]
    fn test_new_blob_validation() {
        let (sampling_relay, _) = new_sampling_relay();

        // No validation because outside window
        // (200 - 100) > (10 / 2)
        let no_validation = new_blob_validation::<SignedMantleTx>(
            100.into(),
            &BlockType::Recent,
            200.into(),
            10.try_into().unwrap(),
            &sampling_relay,
        );
        assert!(no_validation.is_none());

        // RecentBlobValidation for a recent block within window
        // (103 - 100) <= (10 / 2)
        let recent_validation = new_blob_validation::<SignedMantleTx>(
            100.into(),
            &BlockType::Recent,
            103.into(),
            10.try_into().unwrap(),
            &sampling_relay,
        );
        assert!(
            recent_validation
                .unwrap()
                .as_ref()
                .as_any()
                .is::<RecentBlobValidation<da::BlobId>>()
        );

        // HistoricBlobValidation for a historic block within window
        // (103 - 100) <= (10 / 2)
        let historic_validation = new_blob_validation::<SignedMantleTx>(
            100.into(),
            &BlockType::Historic,
            103.into(),
            10.try_into().unwrap(),
            &sampling_relay,
        );
        assert!(
            historic_validation
                .unwrap()
                .as_ref()
                .as_any()
                .is::<HistoricBlobValidation>()
        );
    }

    fn new_sampling_relay() -> (
        SamplingRelay<da::BlobId>,
        mpsc::Receiver<DaSamplingServiceMsg<da::BlobId>>,
    ) {
        let (sender, receiver) = mpsc::channel(10);
        (SamplingRelay::new(sender), receiver)
    }
}
