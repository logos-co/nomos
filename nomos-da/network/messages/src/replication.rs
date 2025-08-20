use nomos_core::{
    da::BlobId,
    mantle::{SignedMantleTx, Transaction as _},
};
use serde::{Deserialize, Serialize};

use crate::{
    common::{Share, ShareRequest},
    SubnetworkId,
};

#[repr(C)]
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReplicationRequest {
    Share(ShareRequest),
    Tx(SignedMantleTx),
}

impl ReplicationRequest {
    #[must_use]
    pub const fn new_share(share: Share) -> Self {
        Self::Share(ShareRequest {
            subnetwork_id: share.data.share_idx,
            share,
        })
    }

    #[must_use]
    pub fn id(&self) -> ReplicationResponseId {
        match self {
            Self::Share(share) => (share.share.blob_id, share.subnetwork_id).into(),
            Self::Tx(tx) => tx.into(),
        }
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

impl From<&SignedMantleTx> for ReplicationResponseId {
    fn from(tx: &SignedMantleTx) -> Self {
        let mut id = [0; 34];
        id[..32].copy_from_slice(&tx.hash().0);
        Self(id)
    }
}
