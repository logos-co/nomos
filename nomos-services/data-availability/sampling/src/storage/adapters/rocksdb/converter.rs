use bytes::Bytes;
use kzgrs_backend::common::share::{DaLightShare, DaShare, DaSharesCommitments};
use nomos_core::{
    da::{BlobId, blob::Share},
    mantle::SignedMantleTx,
};
use nomos_storage::{
    api::da::{DaConverter, StorageDaApi},
    backends::{SerdeOp, rocksdb::RocksBackend},
};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct DaStorageConverter;

impl DaConverter<RocksBackend> for DaStorageConverter {
    type Share = DaShare;
    type Tx = SignedMantleTx;
    type Error = nomos_core::codec::Error;

    fn blob_id_to_storage(blob_id: BlobId) -> Result<BlobId, Self::Error> {
        Ok(blob_id)
    }

    fn blob_id_from_storage(blob_id: BlobId) -> Result<BlobId, Self::Error> {
        Ok(blob_id)
    }

    fn share_index_to_storage(
        share_index: <DaShare as Share>::ShareIndex,
    ) -> Result<<RocksBackend as StorageDaApi>::ShareIndex, Self::Error> {
        Ok(share_index)
    }

    fn share_index_from_storage(
        share_index: <RocksBackend as StorageDaApi>::ShareIndex,
    ) -> Result<<DaShare as Share>::ShareIndex, Self::Error> {
        Ok(share_index)
    }

    fn share_to_storage(service_share: DaLightShare) -> Result<Bytes, Self::Error> {
        <DaLightShare as SerdeOp>::serialize(&service_share)
    }

    fn share_from_storage(backend_share: Bytes) -> Result<DaLightShare, Self::Error> {
        <DaLightShare as SerdeOp>::deserialize(&backend_share)
    }

    fn commitments_to_storage(
        service_commitments: DaSharesCommitments,
    ) -> Result<Bytes, Self::Error> {
        <DaSharesCommitments as SerdeOp>::serialize(&service_commitments)
    }

    fn commitments_from_storage(
        backend_commitments: Bytes,
    ) -> Result<DaSharesCommitments, Self::Error> {
        <DaSharesCommitments as SerdeOp>::deserialize(&backend_commitments)
    }

    fn tx_to_storage(
        service_tx: SignedMantleTx,
    ) -> Result<<RocksBackend as StorageDaApi>::Tx, Self::Error> {
        <SignedMantleTx as SerdeOp>::serialize(&service_tx)
    }

    fn tx_from_storage(
        backend_tx: <RocksBackend as StorageDaApi>::Tx,
    ) -> Result<SignedMantleTx, Self::Error> {
        <SignedMantleTx as SerdeOp>::deserialize(&backend_tx)
    }
}
