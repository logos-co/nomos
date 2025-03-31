use nomos_core::da::BlobId;
use serde::{Deserialize, Serialize};

use crate::{common::Share, SubnetworkId};

#[repr(C)]
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplicationRequest {
    pub share: Share,
    pub subnetwork_id: SubnetworkId,
}

impl ReplicationRequest {
    #[must_use]
    pub const fn new(share: Share, subnetwork_id: SubnetworkId) -> Self {
        Self {
            share,
            subnetwork_id,
        }
    }

    #[must_use]
    pub fn id(&self) -> ReplicationResponseId {
        (self.share.blob_id, self.subnetwork_id).into()
    }
}

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct ReplicationResponseId([u8; 34]);

impl From<(BlobId, SubnetworkId)> for ReplicationResponseId {
    fn from((blob_id, subnetwork_id): (BlobId, SubnetworkId)) -> Self {
        let mut id = [0; 34];
        id[..32].copy_from_slice(&blob_id);
        id[32..].copy_from_slice(&subnetwork_id.to_be_bytes());
        Self(id)
    }
}
