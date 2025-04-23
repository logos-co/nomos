use std::{collections::HashSet, hash::Hash};

pub use common_http_client::{BasicAuthCredentials, CommonHttpClient, Error};
use futures::Stream;
use nomos_core::da::{blob::Share, BlobId};
use nomos_http_api_common::{paths, types::DispersalRequest};
use reqwest::Url;
use serde::{de::DeserializeOwned, Serialize};

#[derive(Clone)]
pub struct ExecutorHttpClient {
    client: CommonHttpClient,
}

impl ExecutorHttpClient {
    #[must_use]
    pub fn new(basic_auth: Option<BasicAuthCredentials>) -> Self {
        Self {
            client: CommonHttpClient::new(basic_auth),
        }
    }

    /// Send a `Blob` to be dispersed
    pub async fn publish_blob<Metadata>(
        &self,
        base_url: Url,
        data: Vec<u8>,
        metadata: Metadata,
    ) -> Result<BlobId, Error>
    where
        Metadata: Serialize + Send + Sync,
    {
        let req = DispersalRequest { data, metadata };
        let path = paths::DISPERSE_DATA.trim_start_matches('/');
        let request_url = base_url.join(path).map_err(Error::Url)?;

        self.client
            .post::<DispersalRequest<Metadata>, BlobId>(request_url, &req)
            .await
    }

    /// Get the commitments for a specific `BlobId`
    pub async fn get_commitments<S>(
        &self,
        base_url: Url,
        blob_id: S::BlobId,
    ) -> Result<Option<S::SharesCommitments>, Error>
    where
        S: Share + Send,
        <S as Share>::BlobId: Serialize + Send + Sync,
        <S as Share>::SharesCommitments: DeserializeOwned + Send + Sync,
    {
        self.client.get_commitments::<S>(base_url, blob_id).await
    }

    /// Get share by blob id and share index
    pub async fn get_share<S, C>(
        &self,
        base_url: Url,
        blob_id: S::BlobId,
        share_idx: S::ShareIndex,
    ) -> Result<Option<C>, Error>
    where
        C: DeserializeOwned + Send + Sync,
        S: Share + DeserializeOwned + Send + Sync,
        <S as Share>::BlobId: serde::Serialize + Send + Sync,
        <S as Share>::ShareIndex: serde::Serialize + Send + Sync,
    {
        self.client
            .get_share::<S, C>(base_url, blob_id, share_idx)
            .await
    }

    pub async fn get_shares<B>(
        &self,
        base_url: Url,
        blob_id: B::BlobId,
        requested_shares: HashSet<B::ShareIndex>,
        filter_shares: HashSet<B::ShareIndex>,
        return_available: bool,
    ) -> Result<impl Stream<Item = B::LightShare>, Error>
    where
        B: Share,
        <B as Share>::BlobId: serde::Serialize + Send + Sync,
        <B as Share>::ShareIndex: serde::Serialize + DeserializeOwned + Eq + Hash + Send + Sync,
        <B as Share>::LightShare: DeserializeOwned + Send + Sync,
    {
        self.client
            .get_shares::<B>(
                base_url,
                blob_id,
                requested_shares,
                filter_shares,
                return_available,
            )
            .await
    }
}
