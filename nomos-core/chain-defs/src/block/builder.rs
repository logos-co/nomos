use std::hash::Hash;

use blake2::digest::Digest as _;
use indexmap::IndexSet;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    block::Block,
    crypto::Blake2b,
    header::Builder,
    tx::{Transaction, TxSelect},
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
pub struct BlockBuilder<Tx, TxSelector> {
    tx_selector: TxSelector,
    header_builder: Builder,
    txs: Option<Box<dyn Iterator<Item = Tx>>>,
}

impl<Tx, TxSelector> BlockBuilder<Tx, TxSelector>
where
    Tx: Transaction + Clone + Eq + Hash + Serialize + DeserializeOwned,
    TxSelector: TxSelect<Tx = Tx>,
{
    pub fn new(tx_selector: TxSelector, header_builder: Builder) -> Self {
        Self {
            tx_selector,
            header_builder,
            txs: None,
        }
    }

    #[must_use]
    pub fn with_transactions(mut self, txs: impl Iterator<Item = Tx> + 'static) -> Self {
        self.txs = Some(Box::new(txs));
        self
    }

    pub fn build(self) -> Result<Block<Tx>, String> {
        if let Self {
            tx_selector,
            header_builder,
            txs: Some(txs),
        } = self
        {
            let txs = tx_selector.select_tx_from(txs).collect::<IndexSet<_>>();

            let serialized_content = wire::serialize(&txs).unwrap();
            let content_size = u32::try_from(serialized_content.len()).map_err(|_| {
                format!(
                    "Content is too big: {} out of {} max",
                    serialized_content.len(),
                    u32::MAX
                )
            })?;
            let content_id = <[u8; 32]>::from(Blake2b::digest(&serialized_content)).into();

            let header = header_builder.build(content_id, content_size);

            Ok(Block {
                header,
                cl_transactions: txs,
            })
        } else {
            Err("incomplete block".to_owned())
        }
    }
}
