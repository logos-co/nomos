use std::collections::HashSet;

use nomos_core::da::blob::Share;
use overwatch::DynError;
use tokio::sync::oneshot::Sender;

use crate::{
    api::{
        da::{DaConverter, StorageDaApi},
        StorageApiRequest, StorageBackendApi, StorageOperation,
    },
    backends::StorageBackend,
    StorageMsg, StorageServiceError,
};

pub enum DaApiRequest<B: StorageBackend> {
    GetLightShare {
        blob_id: <B as StorageDaApi>::BlobId,
        share_idx: <B as StorageDaApi>::ShareIndex,
        response_tx: Sender<Option<<B as StorageDaApi>::Share>>,
    },
    GetLightShareIndexes {
        blob_id: <B as StorageDaApi>::BlobId,
        response_tx: Sender<Option<HashSet<<B as StorageDaApi>::ShareIndex>>>,
    },
    GetBlobLightShares {
        blob_id: <B as StorageDaApi>::BlobId,
        response_tx: Sender<Option<Vec<<B as StorageDaApi>::Share>>>,
    },
    StoreLightShare {
        blob_id: <B as StorageDaApi>::BlobId,
        share_idx: <B as StorageDaApi>::ShareIndex,
        light_share: <B as StorageDaApi>::Share,
    },
    GetSharedCommitments {
        blob_id: <B as StorageDaApi>::BlobId,
        response_tx: Sender<Option<<B as StorageDaApi>::Commitments>>,
    },
    StoreSharedCommitments {
        blob_id: <B as StorageDaApi>::BlobId,
        shared_commitments: <B as StorageDaApi>::Commitments,
    },
}

impl<B> StorageOperation<B> for DaApiRequest<B>
where
    B: StorageBackend + StorageBackendApi,
{
    async fn execute(self, backend: &mut B) -> Result<(), StorageServiceError> {
        match self {
            Self::GetLightShare {
                blob_id,
                share_idx,
                response_tx,
            } => handle_get_light_share(backend, blob_id, share_idx, response_tx).await,
            Self::StoreLightShare {
                blob_id,
                share_idx,
                light_share,
            } => handle_store_light_share(backend, blob_id, share_idx, light_share).await,
            Self::GetSharedCommitments {
                blob_id,
                response_tx,
            } => handle_get_shared_commitments(backend, blob_id, response_tx).await,
            Self::StoreSharedCommitments {
                blob_id,
                shared_commitments,
            } => handle_store_shared_commitments(backend, blob_id, shared_commitments).await,
            Self::GetLightShareIndexes {
                blob_id,
                response_tx,
            } => handle_get_share_indexes(backend, blob_id, response_tx).await,
            Self::GetBlobLightShares {
                blob_id,
                response_tx,
            } => handle_get_blob_light_shares(backend, blob_id, response_tx).await,
        }
    }
}

async fn handle_get_shared_commitments<B: StorageBackend>(
    backend: &mut B,
    blob_id: <B as StorageDaApi>::BlobId,
    response_tx: Sender<Option<<B as StorageDaApi>::Commitments>>,
) -> Result<(), StorageServiceError> {
    let result = backend
        .get_shared_commitments(blob_id)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))?;

    if response_tx.send(result).is_err() {
        return Err(StorageServiceError::ReplyError {
            message: "Failed to send reply for get shared commitments request".to_owned(),
        });
    }
    Ok(())
}

async fn handle_get_light_share<B: StorageBackend>(
    backend: &mut B,
    blob_id: B::BlobId,
    share_idx: B::ShareIndex,
    response_tx: Sender<Option<B::Share>>,
) -> Result<(), StorageServiceError> {
    let result = backend
        .get_light_share(blob_id, share_idx)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))?;

    if response_tx.send(result).is_err() {
        return Err(StorageServiceError::ReplyError {
            message: "Failed to send reply for get light share request".to_owned(),
        });
    }

    Ok(())
}

async fn handle_get_blob_light_shares<B: StorageBackend>(
    backend: &mut B,
    blob_id: <B as StorageDaApi>::BlobId,
    response_tx: Sender<Option<Vec<<B as StorageDaApi>::Share>>>,
) -> Result<(), StorageServiceError> {
    let result = backend
        .get_blob_light_shares(blob_id)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))?;

    if response_tx.send(result).is_err() {
        return Err(StorageServiceError::ReplyError {
            message: "Failed to send reply for get blob light shares request".to_owned(),
        });
    }
    Ok(())
}

async fn handle_get_share_indexes<B: StorageBackend>(
    backend: &mut B,
    blob_id: <B as StorageDaApi>::BlobId,
    response_tx: Sender<Option<HashSet<<B as StorageDaApi>::ShareIndex>>>,
) -> Result<(), StorageServiceError> {
    let result = backend
        .get_blob_share_indices(blob_id)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))?;

    if response_tx.send(result).is_err() {
        return Err(StorageServiceError::ReplyError {
            message: "Failed to send reply for get light share indexes request".to_owned(),
        });
    }
    Ok(())
}

async fn handle_store_light_share<B: StorageBackend>(
    backend: &mut B,
    blob_id: B::BlobId,
    share_idx: B::ShareIndex,
    light_share: B::Share,
) -> Result<(), StorageServiceError> {
    backend
        .store_light_share(blob_id, share_idx, light_share)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))
}

async fn handle_store_shared_commitments<B: StorageBackend>(
    backend: &mut B,
    blob_id: B::BlobId,
    shared_commitments: B::Commitments,
) -> Result<(), StorageServiceError> {
    backend
        .store_shared_commitments(blob_id, shared_commitments)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))
}

impl<Api: StorageBackend> StorageMsg<Api> {
    pub fn get_light_share_request<Converter: DaConverter<Api>>(
        blob_id: <<Converter as DaConverter<Api>>::Share as Share>::BlobId,
        share_idx: <<Converter as DaConverter<Api>>::Share as Share>::ShareIndex,
        response_tx: Sender<Option<<Api as StorageDaApi>::Share>>,
    ) -> Result<Self, DynError> {
        let blob_id = Converter::blob_id_to_storage(blob_id).map_err(Into::<DynError>::into)?;
        let share_idx =
            Converter::share_index_to_storage(share_idx).map_err(Into::<DynError>::into)?;
        Ok(Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetLightShare {
                blob_id,
                share_idx,
                response_tx,
            }),
        })
    }

    pub fn get_blob_light_shares_request<Converter: DaConverter<Api>>(
        blob_id: <<Converter as DaConverter<Api>>::Share as Share>::BlobId,
        response_tx: Sender<Option<Vec<<Api as StorageDaApi>::Share>>>,
    ) -> Result<Self, DynError> {
        let blob_id = Converter::blob_id_to_storage(blob_id).map_err(Into::<DynError>::into)?;
        Ok(Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetBlobLightShares {
                blob_id,
                response_tx,
            }),
        })
    }

    pub fn get_light_share_indexes_request<Converter: DaConverter<Api>>(
        blob_id: <<Converter as DaConverter<Api>>::Share as Share>::BlobId,
        response_tx: Sender<Option<HashSet<<Api as StorageDaApi>::ShareIndex>>>,
    ) -> Result<Self, DynError> {
        let blob_id = Converter::blob_id_to_storage(blob_id).map_err(Into::<DynError>::into)?;
        Ok(Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetLightShareIndexes {
                blob_id,
                response_tx,
            }),
        })
    }

    pub fn store_light_share_request<Converter: DaConverter<Api>>(
        blob_id: <<Converter as DaConverter<Api>>::Share as Share>::BlobId,
        share_idx: <<Converter as DaConverter<Api>>::Share as Share>::ShareIndex,
        light_share: <<Converter as DaConverter<Api>>::Share as Share>::LightShare,
    ) -> Result<Self, DynError> {
        let blob_id = Converter::blob_id_to_storage(blob_id).map_err(Into::<DynError>::into)?;
        let share_idx =
            Converter::share_index_to_storage(share_idx).map_err(Into::<DynError>::into)?;
        let light_share =
            Converter::share_to_storage(light_share).map_err(Into::<DynError>::into)?;

        Ok(Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::StoreLightShare {
                blob_id,
                share_idx,
                light_share,
            }),
        })
    }

    pub fn get_shared_commitments_request<C: DaConverter<Api>>(
        blob_id: <<C as DaConverter<Api>>::Share as Share>::BlobId,
        response_tx: Sender<Option<<Api as StorageDaApi>::Commitments>>,
    ) -> Result<Self, DynError> {
        let blob_id = C::blob_id_to_storage(blob_id).map_err(Into::<DynError>::into)?;
        Ok(Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetSharedCommitments {
                blob_id,
                response_tx,
            }),
        })
    }

    pub fn store_shared_commitments_request<C: DaConverter<Api>>(
        blob_id: <<C as DaConverter<Api>>::Share as Share>::BlobId,
        shared_commitments: <C::Share as Share>::SharesCommitments,
    ) -> Result<Self, DynError> {
        let blob_id = C::blob_id_to_storage(blob_id).map_err(Into::<DynError>::into)?;
        let commitments = C::commitments_to_storage(shared_commitments)?;
        Ok(Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::StoreSharedCommitments {
                blob_id,
                shared_commitments: commitments,
            }),
        })
    }
}
