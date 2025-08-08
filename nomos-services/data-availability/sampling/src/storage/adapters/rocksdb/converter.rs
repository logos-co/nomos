use bytes::Bytes;
use kzgrs_backend::common::share::{DaLightShare, DaShare, DaSharesCommitments};
use nomos_core::{
    da::{blob::Share, BlobId},
    mantle::SignedMantleTx,
};
use nomos_storage::{
    api::da::{DaConverter, StorageDaApi},
    backends::{rocksdb::RocksBackend, StorageSerde},
};

pub struct DaStorageConverter;

impl<SerdeOP> DaConverter<RocksBackend<SerdeOP>> for DaStorageConverter
where
    SerdeOP: StorageSerde + Send + Sync + 'static,
    <SerdeOP as StorageSerde>::Error: Send + Sync + 'static,
{
    type Share = DaShare;
    type Tx = SignedMantleTx;
    type Error = SerdeOP::Error;

    fn blob_id_to_storage(blob_id: BlobId) -> Result<BlobId, Self::Error> {
        Ok(blob_id)
    }

    fn blob_id_from_storage(blob_id: BlobId) -> Result<BlobId, Self::Error> {
        Ok(blob_id)
    }

    fn share_index_to_storage(
        share_index: <DaShare as Share>::ShareIndex,
    ) -> Result<<RocksBackend<SerdeOP> as StorageDaApi>::ShareIndex, Self::Error> {
        Ok(share_index)
    }

    fn share_index_from_storage(
        share_index: <RocksBackend<SerdeOP> as StorageDaApi>::ShareIndex,
    ) -> Result<<DaShare as Share>::ShareIndex, Self::Error> {
        Ok(share_index)
    }

    fn share_to_storage(service_share: DaLightShare) -> Result<Bytes, Self::Error> {
        Ok(SerdeOP::serialize(&service_share))
    }

    fn share_from_storage(backend_share: Bytes) -> Result<DaLightShare, Self::Error> {
        SerdeOP::deserialize(backend_share)
    }

    fn commitments_to_storage(
        service_commitments: DaSharesCommitments,
    ) -> Result<Bytes, Self::Error> {
        Ok(SerdeOP::serialize(&service_commitments))
    }

    fn commitments_from_storage(
        backend_commitments: Bytes,
    ) -> Result<DaSharesCommitments, Self::Error> {
        SerdeOP::deserialize(backend_commitments)
    }

    fn tx_to_storage(
        service_tx: SignedMantleTx,
    ) -> Result<<RocksBackend<SerdeOP> as StorageDaApi>::Tx, Self::Error> {
        Ok(SerdeOP::serialize(&service_tx))
    }

    fn tx_from_storage(
        backend_tx: <RocksBackend<SerdeOP> as StorageDaApi>::Tx,
    ) -> Result<SignedMantleTx, Self::Error> {
        SerdeOP::deserialize(backend_tx)
    }
}
