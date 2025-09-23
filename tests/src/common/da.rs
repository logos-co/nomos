use std::time::Duration;

use chain_service::CryptarchiaInfo;
use common_http_client::Error;
use executor_http_client::ExecutorHttpClient;
use futures::StreamExt as _;
use nomos_core::{
    da::BlobId,
    mantle::{AuthenticatedMantleTx as _, Op},
};
use reqwest::Url;

use crate::{adjust_timeout, nodes::executor::Executor};

pub const APP_ID: &str = "0000000000000000000000000000000000000000000000000000000000000000";
pub const DA_TESTS_TIMEOUT: u64 = 120;
pub async fn disseminate_with_metadata(
    executor: &Executor,
    data: &[u8],
    metadata: kzgrs_backend::dispersal::Metadata,
) -> Result<BlobId, Error> {
    let executor_config = executor.config();
    let backend_address = executor_config.http.backend_settings.address;
    let client = ExecutorHttpClient::new(None);
    let exec_url = Url::parse(&format!("http://{backend_address}")).unwrap();

    client
        .publish_blob(exec_url, [0u8; 32].into(), data.to_vec(), metadata)
        .await
}

/// `wait_for_blob_onchain` tracks the latest chain updates, if new blocks
/// doesn't contain the provided `blob_id` it will wait for ever.
pub async fn wait_for_blob_onchain(executor: &Executor, blob_id: BlobId) {
    let block_fut = async {
        let mut onchain = false;
        while !onchain {
            let CryptarchiaInfo { tip, .. } = executor.consensus_info().await;
            if let Some(block) = executor.get_block(tip).await
                && block
                    .transactions()
                    .flat_map(|tx| tx.mantle_tx().ops.iter())
                    .filter_map(|op| {
                        if let Op::ChannelBlob(op) = op {
                            Some(op.blob)
                        } else {
                            None
                        }
                    })
                    .any(|blob| blob == blob_id)
            {
                onchain = true;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };

    let timeout = adjust_timeout(Duration::from_secs(DA_TESTS_TIMEOUT));
    assert!(
        (tokio::time::timeout(timeout, block_fut).await).is_ok(),
        "timed out waiting for blob shares"
    );
}

pub async fn wait_for_shares_number(executor: &Executor, blob_id: BlobId, num_shares: usize) {
    let shares_fut = async {
        let mut got_shares = 0;
        while got_shares < num_shares {
            let shares_result = executor
                .get_shares(blob_id, [].into(), [].into(), true)
                .await;
            if let Ok(shares_stream) = shares_result {
                got_shares = shares_stream.collect::<Vec<_>>().await.len();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };

    let timeout = adjust_timeout(Duration::from_secs(DA_TESTS_TIMEOUT));
    assert!(
        (tokio::time::timeout(timeout, shares_fut).await).is_ok(),
        "timed out waiting for blob shares"
    );
}
