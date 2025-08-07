use std::collections::{HashMap, HashSet};

use multiaddr::Multiaddr;
use nomos_core::block::BlockNumber;
use overwatch::DynError;
use tokio::sync::oneshot::Sender;

use crate::{
    api::{
        da::{
            DaConverter, ServiceBlobId, ServiceLightShare, ServiceShareIndex,
            ServiceSharedCommitments, StorageDaApi,
        },
        StorageApiRequest, StorageBackendApi, StorageOperation,
    },
    backends::StorageBackend,
    StorageMsg, StorageServiceError,
};

type AssignationsMap<Backend> =
    HashMap<<Backend as StorageDaApi>::NetworkId, HashSet<<Backend as StorageDaApi>::Id>>;

type AssignationsResponse<Backend> = AssignationsMap<Backend>;

type AddressbookRequest<Backend> = HashMap<<Backend as StorageDaApi>::Id, Multiaddr>;

pub enum DaApiRequest<Backend: StorageBackend> {
    GetLightShare {
        blob_id: <Backend as StorageDaApi>::BlobId,
        share_idx: <Backend as StorageDaApi>::ShareIndex,
        response_tx: Sender<Option<<Backend as StorageDaApi>::Share>>,
    },
    GetLightShareIndexes {
        blob_id: <Backend as StorageDaApi>::BlobId,
        response_tx: Sender<Option<HashSet<<Backend as StorageDaApi>::ShareIndex>>>,
    },
    GetBlobLightShares {
        blob_id: <Backend as StorageDaApi>::BlobId,
        response_tx: Sender<Option<Vec<<Backend as StorageDaApi>::Share>>>,
    },
    StoreLightShare {
        blob_id: <Backend as StorageDaApi>::BlobId,
        share_idx: <Backend as StorageDaApi>::ShareIndex,
        light_share: <Backend as StorageDaApi>::Share,
    },
    GetSharedCommitments {
        blob_id: <Backend as StorageDaApi>::BlobId,
        response_tx: Sender<Option<<Backend as StorageDaApi>::Commitments>>,
    },
    StoreSharedCommitments {
        blob_id: <Backend as StorageDaApi>::BlobId,
        shared_commitments: <Backend as StorageDaApi>::Commitments,
    },
    GetAssignations {
        block_number: BlockNumber,
        response_tx: Sender<Option<AssignationsMap<Backend>>>,
    },
    StoreAssignations {
        block_number: BlockNumber,
        assignations: AssignationsMap<Backend>,
    },
    StoreAddresses {
        ids: AddressbookRequest<Backend>,
    },
    GetAddress {
        id: Backend::Id,
        response_tx: Sender<Option<Multiaddr>>,
    },
}

impl<Backend> StorageOperation<Backend> for DaApiRequest<Backend>
where
    Backend: StorageBackend + StorageBackendApi,
{
    async fn execute(self, backend: &mut Backend) -> Result<(), StorageServiceError> {
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
            Self::StoreAssignations {
                block_number,
                assignations,
            } => handle_store_assignations(backend, block_number, assignations).await,
            Self::GetAssignations {
                block_number,
                response_tx,
            } => handle_get_assignations(backend, block_number, response_tx).await,
            Self::StoreAddresses { ids } => handle_store_addresses(backend, ids).await,
            Self::GetAddress { id, response_tx } => {
                handle_get_address(backend, id, response_tx).await
            }
        }
    }
}

async fn handle_get_shared_commitments<Backend: StorageBackend>(
    backend: &mut Backend,
    blob_id: <Backend as StorageDaApi>::BlobId,
    response_tx: Sender<Option<<Backend as StorageDaApi>::Commitments>>,
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

async fn handle_get_light_share<Backend: StorageBackend>(
    backend: &mut Backend,
    blob_id: Backend::BlobId,
    share_idx: Backend::ShareIndex,
    response_tx: Sender<Option<Backend::Share>>,
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

async fn handle_get_blob_light_shares<Backend: StorageBackend>(
    backend: &mut Backend,
    blob_id: <Backend as StorageDaApi>::BlobId,
    response_tx: Sender<Option<Vec<<Backend as StorageDaApi>::Share>>>,
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

async fn handle_get_share_indexes<Backend: StorageBackend>(
    backend: &mut Backend,
    blob_id: <Backend as StorageDaApi>::BlobId,
    response_tx: Sender<Option<HashSet<<Backend as StorageDaApi>::ShareIndex>>>,
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

async fn handle_store_light_share<Backend: StorageBackend>(
    backend: &mut Backend,
    blob_id: Backend::BlobId,
    share_idx: Backend::ShareIndex,
    light_share: Backend::Share,
) -> Result<(), StorageServiceError> {
    backend
        .store_light_share(blob_id, share_idx, light_share)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))
}

async fn handle_store_shared_commitments<Backend: StorageBackend>(
    backend: &mut Backend,
    blob_id: Backend::BlobId,
    shared_commitments: Backend::Commitments,
) -> Result<(), StorageServiceError> {
    backend
        .store_shared_commitments(blob_id, shared_commitments)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))
}

async fn handle_store_assignations<Backend: StorageBackend>(
    backend: &mut Backend,
    block_number: BlockNumber,
    assignations: AssignationsMap<Backend>,
) -> Result<(), StorageServiceError> {
    backend
        .store_assignations(block_number, assignations)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))
}

async fn handle_get_assignations<Backend: StorageBackend>(
    backend: &mut Backend,
    block_number: BlockNumber,
    response_tx: Sender<Option<AssignationsResponse<Backend>>>,
) -> Result<(), StorageServiceError> {
    let result = backend
        .get_assignations(block_number)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))?;

    if response_tx.send(Some(result)).is_err() {
        return Err(StorageServiceError::ReplyError {
            message: "Failed to send reply for get assignations request".to_owned(),
        });
    }
    Ok(())
}

async fn handle_store_addresses<Backend: StorageBackend>(
    backend: &mut Backend,
    ids: AddressbookRequest<Backend>,
) -> Result<(), StorageServiceError> {
    backend
        .store_addresses(ids)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))
}

async fn handle_get_address<Backend: StorageBackend>(
    backend: &mut Backend,
    id: Backend::Id,
    response_tx: Sender<Option<Multiaddr>>,
) -> Result<(), StorageServiceError> {
    let result = backend
        .get_address(id)
        .await
        .map_err(|e| StorageServiceError::BackendError(e.into()))?;

    if response_tx.send(result).is_err() {
        return Err(StorageServiceError::ReplyError {
            message: "Failed to send reply for get address request".to_owned(),
        });
    }
    Ok(())
}

impl<Backend: StorageBackend> StorageMsg<Backend> {
    pub fn get_light_share_request<Converter: DaConverter<Backend>>(
        blob_id: ServiceBlobId<Converter, Backend>,
        share_idx: ServiceShareIndex<Converter, Backend>,
        response_tx: Sender<Option<<Backend as StorageDaApi>::Share>>,
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

    pub fn get_blob_light_shares_request<Converter: DaConverter<Backend>>(
        blob_id: ServiceBlobId<Converter, Backend>,
        response_tx: Sender<Option<Vec<<Backend as StorageDaApi>::Share>>>,
    ) -> Result<Self, DynError> {
        let blob_id = Converter::blob_id_to_storage(blob_id).map_err(Into::<DynError>::into)?;
        Ok(Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetBlobLightShares {
                blob_id,
                response_tx,
            }),
        })
    }

    pub fn get_light_share_indexes_request<Converter: DaConverter<Backend>>(
        blob_id: ServiceBlobId<Converter, Backend>,
        response_tx: Sender<Option<HashSet<<Backend as StorageDaApi>::ShareIndex>>>,
    ) -> Result<Self, DynError> {
        let blob_id = Converter::blob_id_to_storage(blob_id).map_err(Into::<DynError>::into)?;
        Ok(Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetLightShareIndexes {
                blob_id,
                response_tx,
            }),
        })
    }

    pub fn store_light_share_request<Converter: DaConverter<Backend>>(
        blob_id: ServiceBlobId<Converter, Backend>,
        share_idx: ServiceShareIndex<Converter, Backend>,
        light_share: ServiceLightShare<Converter, Backend>,
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

    pub fn get_shared_commitments_request<Converter: DaConverter<Backend>>(
        blob_id: ServiceBlobId<Converter, Backend>,
        response_tx: Sender<Option<<Backend as StorageDaApi>::Commitments>>,
    ) -> Result<Self, DynError> {
        let blob_id = Converter::blob_id_to_storage(blob_id).map_err(Into::<DynError>::into)?;
        Ok(Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetSharedCommitments {
                blob_id,
                response_tx,
            }),
        })
    }

    pub fn store_shared_commitments_request<Converter: DaConverter<Backend>>(
        blob_id: ServiceBlobId<Converter, Backend>,
        shared_commitments: ServiceSharedCommitments<Converter, Backend>,
    ) -> Result<Self, DynError> {
        let blob_id = Converter::blob_id_to_storage(blob_id).map_err(Into::<DynError>::into)?;
        let shared_commitments = Converter::commitments_to_storage(shared_commitments)?;
        Ok(Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::StoreSharedCommitments {
                blob_id,
                shared_commitments,
            }),
        })
    }

    #[must_use]
    pub const fn get_assignations_request(
        block_number: BlockNumber,
        response_tx: Sender<Option<AssignationsResponse<Backend>>>,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetAssignations {
                block_number,
                response_tx,
            }),
        }
    }

    #[must_use]
    pub const fn store_assignations_request(
        block_number: BlockNumber,
        assignations: AssignationsMap<Backend>,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::StoreAssignations {
                block_number,
                assignations,
            }),
        }
    }

    #[must_use]
    pub const fn store_addresses_request(ids: AddressbookRequest<Backend>) -> Self {
        Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::StoreAddresses { ids }),
        }
    }

    #[must_use]
    pub const fn get_address_request(
        id: Backend::Id,
        response_tx: Sender<Option<Multiaddr>>,
    ) -> Self {
        Self::Api {
            request: StorageApiRequest::Da(DaApiRequest::GetAddress { id, response_tx }),
        }
    }
}
