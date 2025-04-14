use std::{collections::HashSet, fmt::Debug, hash::Hash, sync::Arc};

use futures::{Stream, StreamExt};
use nomos_core::da::blob::Share;
use nomos_da_messages::http::da::{
    DASharesCommitmentsRequest, DaSamplingRequest, GetSharesRequest,
};
use nomos_http_api_common::paths::{DA_GET_LIGHT_SHARE, DA_GET_SHARES, DA_GET_SHARES_COMMITMENTS};
use reqwest::{Client, ClientBuilder, RequestBuilder, StatusCode, Url};
use serde::{de::DeserializeOwned, Serialize};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Internal server error: {0}")]
    Server(String),
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error(transparent)]
    Url(#[from] url::ParseError),
}

#[derive(Clone)]
pub struct BasicAuthCredentials {
    username: String,
    password: Option<String>,
}

impl BasicAuthCredentials {
    #[must_use]
    pub const fn new(username: String, password: Option<String>) -> Self {
        Self { username, password }
    }
}

#[derive(Clone)]
pub struct CommonHttpClient {
    client: Arc<Client>,
    basic_auth: Option<BasicAuthCredentials>,
}

impl CommonHttpClient {
    #[must_use]
    pub fn new(basic_auth: Option<BasicAuthCredentials>) -> Self {
        let client = ClientBuilder::new()
            .build()
            .expect("Client from default settings should be able to build");
        Self {
            client: Arc::new(client),
            basic_auth,
        }
    }
    pub async fn post<Req, Res>(&self, request_url: Url, request_body: &Req) -> Result<Res, Error>
    where
        Req: Serialize + ?Sized + Send + Sync,
        Res: DeserializeOwned + Send + Sync,
    {
        let request = self.client.post(request_url).json(request_body);
        self.execute_request::<Res>(request).await
    }

    pub async fn get<Req, Res>(&self, request_url: Url, request_body: &Req) -> Result<Res, Error>
    where
        Req: Serialize + ?Sized + Send + Sync,
        Res: DeserializeOwned + Send + Sync,
    {
        let request = self.client.get(request_url).json(request_body);
        self.execute_request::<Res>(request).await
    }

    async fn execute_request<Res: DeserializeOwned>(
        &self,
        mut request: RequestBuilder,
    ) -> Result<Res, Error> {
        if let Some(basic_auth) = &self.basic_auth {
            request = request.basic_auth(&basic_auth.username, basic_auth.password.as_deref());
        }

        let response = request.send().await.map_err(Error::Request)?;
        let status = response.status();
        let body = response.text().await.map_err(Error::Request)?;

        match status {
            StatusCode::OK => serde_json::from_str(&body)
                .map_err(|e| Error::Server(format!("Failed to parse response: {e}"))),
            StatusCode::INTERNAL_SERVER_ERROR => Err(Error::Server(body)),
            _ => Err(Error::Server(format!(
                "Unexpected response [{status}]: {body}",
            ))),
        }
    }

    /// Get the commitments for a Blob
    pub async fn get_commitments<S>(
        &self,
        base_url: Url,
        blob_id: S::BlobId,
    ) -> Result<Option<S::SharesCommitments>, Error>
    where
        S: Share,
        S::SharesCommitments: DeserializeOwned + Send + Sync,
        <S as Share>::BlobId: serde::Serialize + Send + Sync,
    {
        let request: DASharesCommitmentsRequest<S> = DASharesCommitmentsRequest { blob_id };
        let path = DA_GET_SHARES_COMMITMENTS.trim_start_matches('/');
        let request_url = base_url.join(path).map_err(Error::Url)?;
        self.get(request_url, &request).await
    }

    /// Get blob by blob id and column index
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
        let request: DaSamplingRequest<S> = DaSamplingRequest { blob_id, share_idx };
        let path = DA_GET_LIGHT_SHARE.trim_start_matches('/');
        let request_url = base_url.join(path).map_err(Error::Url)?;
        self.get(request_url, &request).await
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
        let request: GetSharesRequest<B> = GetSharesRequest {
            blob_id,
            requested_shares,
            filter_shares,
            return_available,
        };
        let request_url = base_url
            .join(DA_GET_SHARES.trim_start_matches('/'))
            .map_err(Error::Url)?;
        let mut request = self.client.get(request_url).json(&request);

        if let Some(basic_auth) = &self.basic_auth {
            request = request.basic_auth(&basic_auth.username, basic_auth.password.as_deref());
        }

        let response = request.send().await.map_err(Error::Request)?;
        let status = response.status();

        let shares_stream = response.bytes_stream().filter_map(|item| async move {
            let bytes = item.ok()?;
            serde_json::from_slice::<B::LightShare>(&bytes).ok()
        });
        match status {
            StatusCode::OK => Ok(shares_stream),
            StatusCode::INTERNAL_SERVER_ERROR => Err(Error::Server("Error".to_owned())),
            _ => Err(Error::Server(format!("Unexpected response [{status}]",))),
        }
    }
}
