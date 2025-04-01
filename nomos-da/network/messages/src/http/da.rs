use std::{collections::HashSet, hash::Hash, ops::Range};

use nomos_core::da::blob::{metadata::Metadata, Share};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

// Shared types for http requests. Probably better part of upcoming `nomos-lib`

#[derive(Serialize, Deserialize)]
pub struct GetRangeReq<V: Metadata>
where
    <V as Metadata>::AppId: Serialize + DeserializeOwned,
    <V as Metadata>::Index: Serialize + DeserializeOwned,
{
    pub app_id: <V as Metadata>::AppId,
    pub range: Range<<V as Metadata>::Index>,
}

#[derive(Serialize, Deserialize)]
pub struct DASharesCommitmentsRequest<S: Share> {
    pub blob_id: S::BlobId,
}

#[derive(Serialize, Deserialize)]
pub struct DaSamplingRequest<S: Share> {
    pub blob_id: S::BlobId,
    pub share_idx: S::ShareIndex,
}

#[derive(Serialize, Deserialize)]
pub struct GetSharesRequest<B: Share>
where
    <B as Share>::ShareIndex: Serialize + DeserializeOwned + Eq + Hash,
{
    pub blob_id: B::BlobId,
    pub requested_shares: HashSet<B::ShareIndex>,
    pub filter_shares: HashSet<B::ShareIndex>,
    pub return_available: bool,
}
