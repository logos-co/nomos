use std::time::Duration;

use chain_service::CryptarchiaInfo;
use futures::FutureExt as _;
use nomos_core::{block::Block, header::HeaderId, mantle::SignedMantleTx};
use nomos_http_api_common::paths;
use reqwest::{Client as ReqwestClient, Url};

use super::{
    context::{ClientError, NodeClient},
    plan::BoxFuture,
};
use crate::topology::GeneratedNodeConfig;

pub struct HttpNodeClient {
    descriptor: GeneratedNodeConfig,
    base_url: Url,
    raw: ReqwestClient,
}

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

impl HttpNodeClient {
    #[must_use]
    pub fn new(descriptor: GeneratedNodeConfig, base_url: Url) -> Self {
        Self {
            descriptor,
            base_url,
            raw: ReqwestClient::builder()
                .timeout(HTTP_TIMEOUT)
                .connect_timeout(HTTP_CONNECT_TIMEOUT)
                .build()
                .expect("constructing reqwest client"),
        }
    }

    fn make_url(&self, path: &str) -> Result<Url, ClientError> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .map_err(|err| ClientError::Http(err.to_string()))
    }
}

impl NodeClient for HttpNodeClient {
    fn descriptor(&self) -> &GeneratedNodeConfig {
        &self.descriptor
    }

    fn submit_transaction(&self, tx: SignedMantleTx) -> BoxFuture<'_, Result<(), ClientError>> {
        async move {
            let url = self.make_url(paths::MEMPOOL_ADD_TX)?;
            let response = self.raw.post(url).json(&tx).send().await?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(ClientError::Http(format!(
                    "unexpected status {}",
                    response.status()
                )))
            }
        }
        .boxed()
    }

    fn consensus_info(&self) -> BoxFuture<'_, Result<CryptarchiaInfo, ClientError>> {
        async move {
            let url = self.make_url(paths::CRYPTARCHIA_INFO)?;
            let response = self.raw.get(url).send().await?;
            if !response.status().is_success() {
                return Err(ClientError::Http(format!(
                    "unexpected status {}",
                    response.status()
                )));
            }
            let info = response.json::<CryptarchiaInfo>().await?;
            Ok(info)
        }
        .boxed()
    }

    fn get_block(
        &self,
        id: HeaderId,
    ) -> BoxFuture<'_, Result<Option<Block<SignedMantleTx>>, ClientError>> {
        async move {
            let url = self.make_url(paths::STORAGE_BLOCK)?;
            let response = self.raw.post(url).json(&id).send().await?;
            if response.status().is_success() {
                let block = response.json::<Option<Block<SignedMantleTx>>>().await?;
                Ok(block)
            } else if response.status().is_client_error() {
                Ok(None)
            } else {
                Err(ClientError::Http(format!(
                    "unexpected status {}",
                    response.status()
                )))
            }
        }
        .boxed()
    }
}
