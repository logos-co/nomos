pub mod mock;

use nomos_core::{
    da::BlobId,
    mantle::{ops::channel::Ed25519PublicKey, SignedMantleTx},
};

#[async_trait::async_trait]
pub trait DaWalletAdapter {
    type Error;

    // TODO: Pass relay when wallet service is defined.
    fn new() -> Self;

    fn blob_tx(
        &self,
        blob_id: BlobId,
        blob_size: usize,
        signer: Ed25519PublicKey,
    ) -> Result<SignedMantleTx, Self::Error>;
}
