use blake2::digest::Digest as _;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    block::Block,
    crypto::Hasher,
    da::blob::{info::DispersedBlobInfo, BlobSelect},
    header::Builder,
    mantle::{Transaction, TxSelect},
    wire,
};

/// Wrapper over a block building `new` method than holds intermediary state and
/// can be passed around.
///
/// It also compounds the transaction selection and blob selection heuristics to
/// be used for transaction and blob selection.
///
/// Example:
/// ``` ignore
/// use nomos_core::block::builder::BlockBuilder;
/// let builder: BlockBuilder<(), (), FirstTx, FirstBlob> = {
///     BlockBuilder::new( FirstTx::default(), FirstBlob::default())
///         .with_transactions([tx1].into_iter())
///         .with_blobs([blob1].into_iter())
/// };
/// builder.build().expect("All block attributes should have been set")
/// ```
pub struct BlockBuilder<Tx, Blob, TxSelector, BlobSelector> {
    tx_selector: TxSelector,
    blob_selector: BlobSelector,
    header_builder: Builder,
    txs: Option<Box<dyn Iterator<Item = Tx>>>,
    blobs: Option<Box<dyn Iterator<Item = Blob>>>,
}

impl<Tx, B, TxSelector, BlobSelector> BlockBuilder<Tx, B, TxSelector, BlobSelector>
where
    Tx: Transaction + Clone + Eq + Serialize + DeserializeOwned,
    B: DispersedBlobInfo + Clone + Eq + Serialize + DeserializeOwned,
    TxSelector: TxSelect<Tx = Tx>,
    BlobSelector: BlobSelect<BlobId = B>,
{
    pub fn new(
        tx_selector: TxSelector,
        blob_selector: BlobSelector,
        header_builder: Builder,
    ) -> Self {
        Self {
            tx_selector,
            blob_selector,
            header_builder,
            txs: None,
            blobs: None,
        }
    }

    #[must_use]
    pub fn with_transactions(mut self, txs: impl Iterator<Item = Tx> + 'static) -> Self {
        self.txs = Some(Box::new(txs));
        self
    }

    #[must_use]
    pub fn with_blobs_info(
        mut self,
        blobs_certificates: impl Iterator<Item = B> + 'static,
    ) -> Self {
        self.blobs = Some(Box::new(blobs_certificates));
        self
    }

    pub fn build(self) -> Result<Block<Tx, B>, String> {
        if let Self {
            tx_selector,
            blob_selector,
            header_builder,
            txs: Some(txs),
            blobs: Some(blobs),
        } = self
        {
            let txs = tx_selector.select_tx_from(txs).collect::<Vec<_>>();
            let blobs = blob_selector.select_blob_from(blobs).collect::<Vec<_>>();

            let serialized_content = wire::serialize(&(&txs, &blobs)).unwrap();
            let content_size = u32::try_from(serialized_content.len()).map_err(|_| {
                format!(
                    "Content is too big: {} out of {} max",
                    serialized_content.len(),
                    u32::MAX
                )
            })?;
            let content_id = <[u8; 32]>::from(Hasher::digest(&serialized_content)).into();

            let header = header_builder.build(content_id, content_size);

            Ok(Block {
                header,
                cl_transactions: txs,
                bl_blobs: blobs,
            })
        } else {
            Err("incomplete block".to_owned())
        }
    }
}
