use std::collections::{BTreeSet, HashMap};

use async_trait::async_trait;
use nomos_core::{
    block::{BlockNumber, SessionNumber},
    sdp::{Locator, ProviderId, ServiceType},
};
use rocksdb::Error;
use tracing::{debug, error};

use crate::{
    api::{backend::rocksdb::utils::key_bytes, membership::StorageMembershipApi},
    backends::{rocksdb::RocksBackend, StorageBackend as _, StorageSerde},
};

pub const MEMBERSHIP_ACTIVE_SESSION_PREFIX: &str = "membership/active/";
pub const MEMBERSHIP_FORMING_SESSION_PREFIX: &str = "membership/forming/";
pub const MEMBERSHIP_LATEST_BLOCK_KEY: &str = "membership/latest_block";

#[async_trait]
impl<SerdeOp: StorageSerde + Send + Sync + 'static> StorageMembershipApi for RocksBackend<SerdeOp> {
    type Error = Error;

    async fn save_active_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), Self::Error> {
        let service_bytes = SerdeOp::serialize(service_type);
        let key = key_bytes(MEMBERSHIP_ACTIVE_SESSION_PREFIX, service_bytes);

        let session_data = (session_id, providers);
        let serialized_data = SerdeOp::serialize(session_data);

        match self.store(key, serialized_data).await {
            Ok(()) => {
                debug!(
                    "Successfully stored active session {} for service {:?}",
                    session_id, service_type
                );
                Ok(())
            }
            Err(e) => {
                error!("Failed to store active session: {:?}", e);
                Err(e)
            }
        }
    }

    async fn load_active_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, Self::Error> {
        let service_bytes = SerdeOp::serialize(service_type);
        let key = key_bytes(MEMBERSHIP_ACTIVE_SESSION_PREFIX, service_bytes);

        let data = self.load(&key).await?;

        data.map_or_else(
            || {
                debug!("No active session found for service {:?}", service_type);
                Ok(None)
            },
            |bytes| match SerdeOp::deserialize::<(
                SessionNumber,
                HashMap<ProviderId, BTreeSet<Locator>>,
            )>(bytes)
            {
                Ok(session_data) => {
                    debug!(
                        "Successfully loaded active session for service {:?}",
                        service_type
                    );
                    Ok(Some(session_data))
                }
                Err(e) => {
                    error!("Failed to deserialize active session: {:?}", e);
                    Ok(None)
                }
            },
        )
    }

    async fn save_latest_block(&mut self, block_number: BlockNumber) -> Result<(), Self::Error> {
        let block_bytes = block_number.to_be_bytes();

        match self
            .store(
                MEMBERSHIP_LATEST_BLOCK_KEY.into(),
                block_bytes.to_vec().into(),
            )
            .await
        {
            Ok(()) => {
                debug!("Successfully stored latest block {}", block_number);
                Ok(())
            }
            Err(e) => {
                error!("Failed to store latest block: {:?}", e);
                Err(e)
            }
        }
    }

    async fn load_latest_block(&mut self) -> Result<Option<BlockNumber>, Self::Error> {
        let data = self.load(MEMBERSHIP_LATEST_BLOCK_KEY.as_bytes()).await?;

        match data {
            None => {
                debug!("No latest block found");
                Ok(None)
            }
            Some(bytes) => {
                if bytes.len() != 8 {
                    error!("Invalid block number bytes length: {}", bytes.len());
                    return Ok(None);
                }

                let block_bytes: [u8; 8] = bytes[..8].try_into().unwrap();
                let block_number = BlockNumber::from_be_bytes(block_bytes);
                debug!("Successfully loaded latest block {}", block_number);
                Ok(Some(block_number))
            }
        }
    }

    async fn save_forming_session(
        &mut self,
        service_type: ServiceType,
        session_id: SessionNumber,
        providers: &HashMap<ProviderId, BTreeSet<Locator>>,
    ) -> Result<(), Self::Error> {
        let service_bytes = SerdeOp::serialize(service_type);
        let key = key_bytes(MEMBERSHIP_FORMING_SESSION_PREFIX, service_bytes);

        let session_data = (session_id, providers);
        let serialized_data = SerdeOp::serialize(session_data);

        match self.store(key, serialized_data).await {
            Ok(()) => {
                debug!(
                    "Successfully stored forming session {} for service {:?}",
                    session_id, service_type
                );
                Ok(())
            }
            Err(e) => {
                error!("Failed to store forming session: {:?}", e);
                Err(e)
            }
        }
    }

    async fn load_forming_session(
        &mut self,
        service_type: ServiceType,
    ) -> Result<Option<(SessionNumber, HashMap<ProviderId, BTreeSet<Locator>>)>, Self::Error> {
        let service_bytes = SerdeOp::serialize(service_type);
        let key = key_bytes(MEMBERSHIP_FORMING_SESSION_PREFIX, service_bytes);

        let data = self.load(&key).await?;

        data.map_or_else(
            || {
                debug!("No forming session found for service {:?}", service_type);
                Ok(None)
            },
            |bytes| match SerdeOp::deserialize::<(
                SessionNumber,
                HashMap<ProviderId, BTreeSet<Locator>>,
            )>(bytes)
            {
                Ok(session_data) => {
                    debug!(
                        "Successfully loaded forming session for service {:?}",
                        service_type
                    );
                    Ok(Some(session_data))
                }
                Err(e) => {
                    error!("Failed to deserialize forming session: {:?}", e);
                    Ok(None)
                }
            },
        )
    }
}
