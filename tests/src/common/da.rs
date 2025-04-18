use std::time::Duration;

use common_http_client::Error;
use executor_http_client::ExecutorHttpClient;
use reqwest::Url;

use crate::{adjust_timeout, nodes::executor::Executor};

pub const APP_ID: &str = "fd3384e132ad02a56c78f45547ee40038dc79002b90d29ed90e08eee762ae715";
pub const DA_TESTS_TIMEOUT: u64 = 120;
pub async fn disseminate_with_metadata(
    executor: &Executor,
    data: &[u8],
    metadata: kzgrs_backend::dispersal::Metadata,
) -> Result<(), Error> {
    let executor_config = executor.config();
    let backend_address = executor_config.http.backend_settings.address;
    let client = ExecutorHttpClient::new(None);
    let exec_url = Url::parse(&format!("http://{backend_address}")).unwrap();

    client.publish_blob(exec_url, data.to_vec(), metadata).await
}

pub async fn wait_for_indexed_blob(
    executor: &Executor,
    app_id: [u8; 32],
    from: [u8; 8],
    to: [u8; 8],
    num_subnets: usize,
) {
    let shares_fut = async {
        let mut num_shares = 0;
        while num_shares < num_subnets {
            let executor_shares = executor.get_indexer_range(app_id, from..to).await;
            num_shares = executor_shares
                .into_iter()
                .filter(|(i, _)| i == &from)
                .flat_map(|(_, shares)| shares)
                .count();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };

    let timeout = adjust_timeout(Duration::from_secs(DA_TESTS_TIMEOUT));
    assert!(
        (tokio::time::timeout(timeout, shares_fut).await).is_ok(),
        "timed out waiting for indexed blob"
    );
}
