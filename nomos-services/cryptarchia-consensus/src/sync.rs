use std::{hash::Hash, marker::PhantomData};

use bytes::Bytes;
use cryptarchia_engine::Slot;
use cryptarchia_sync::CryptarchiaSyncError;
use nomos_core::{block::Block, header::HeaderId};
use nomos_ledger::leader_proof::LeaderProof;
use nomos_storage::{backends::StorageBackend, StorageMsg, StorageService};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::Serialize;
use tracing::error;

use crate::{leadership::Leader, Cryptarchia, Error};

pub struct CryptarchiaSync<Storage, Tx, BlobCertificate, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync,
{
    cryptarchia: Cryptarchia,
    leader: Leader,
    storage_relay:
        OutboundRelay<<StorageService<Storage, RuntimeServiceId> as ServiceData>::Message>,
    _marker: PhantomData<(Tx, BlobCertificate)>,
}

impl<Storage, Tx, BlobCertificate, RuntimeServiceId>
    CryptarchiaSync<Storage, Tx, BlobCertificate, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync,
{
    pub const fn new(
        cryptarchia: Cryptarchia,
        leader: Leader,
        storage_relay: OutboundRelay<
            <StorageService<Storage, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self {
        Self {
            cryptarchia,
            leader,
            storage_relay,
            _marker: PhantomData,
        }
    }

    pub fn take(self) -> (Cryptarchia, Leader) {
        (self.cryptarchia, self.leader)
    }
}

#[async_trait::async_trait]
impl<Storage, Tx, BlobCertificate, RuntimeServiceId> cryptarchia_sync::CryptarchiaSync
    for CryptarchiaSync<Storage, Tx, BlobCertificate, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync,
    Tx: Clone + Eq + Hash + Serialize + Send,
    BlobCertificate: Clone + Eq + Hash + Serialize + Send,
{
    type Block = Block<Tx, BlobCertificate>;

    async fn process_block(&mut self, block: Self::Block) -> Result<(), CryptarchiaSyncError> {
        // TODO: DA blob validation

        let header = block.header();
        match self.cryptarchia.try_apply_header(header) {
            Ok(cryptarchia) => {
                self.leader.follow_chain(
                    header.parent(),
                    header.id(),
                    header.leader_proof().nullifier(),
                );

                let key: [u8; 32] = header.id().into();
                let msg = <StorageMsg<_>>::new_store_message(Bytes::copy_from_slice(&key), block);
                if let Err((e, _)) = self.storage_relay.send(msg).await {
                    error!("Could not send block to storage: {e}");
                }
                self.cryptarchia = cryptarchia;
                Ok(())
            }
            Err(
                Error::Ledger(nomos_ledger::LedgerError::ParentNotFound(_))
                | Error::Consensus(cryptarchia_engine::Error::ParentMissing(_)),
            ) => Err(CryptarchiaSyncError::ParentNotFound),
            Err(e) => Err(CryptarchiaSyncError::InvalidBlock(e.into())),
        }
    }

    fn tip_slot(&self) -> Slot {
        self.cryptarchia.tip_state().slot()
    }

    fn has_block(&self, id: &HeaderId) -> bool {
        self.cryptarchia.ledger.state(id).is_some()
    }
}
