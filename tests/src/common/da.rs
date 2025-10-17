use std::time::Duration;

use chain_service::CryptarchiaInfo;
use common_http_client::Error;
use ed25519_dalek::{Signer as _, SigningKey};
use executor_http_client::ExecutorHttpClient;
use futures::StreamExt as _;
use nomos_core::{
    da::BlobId,
    mantle::{
        AuthenticatedMantleTx as _, MantleTx, Op, SignedMantleTx, Transaction as _,
        ledger::Tx as LedgerTx,
        ops::{
            OpProof,
            channel::{ChannelId, MsgId, inscribe::InscriptionOp},
        },
    },
    proofs::zksig::{DummyZkSignature, ZkSignaturePublic},
};
use reqwest::Url;

use crate::{adjust_timeout, nodes::executor::Executor};

pub const APP_ID: &str = "0000000000000000000000000000000000000000000000000000000000000000";
pub const DA_TESTS_TIMEOUT: u64 = 120;
pub async fn disseminate_with_metadata(
    executor: &Executor,
    channel_id: ChannelId,
    data: &[u8],
    metadata: kzgrs_backend::dispersal::Metadata,
) -> Result<BlobId, Error> {
    let executor_config = executor.config();
    let backend_address = executor_config.http.backend_settings.address;
    let client = ExecutorHttpClient::new(None);
    let exec_url = Url::parse(&format!("http://{backend_address}")).unwrap();

    client
        .publish_blob(exec_url, channel_id, data.to_vec(), metadata)
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

/// Sets up a test channel by sending an inscription transaction and waiting for
/// it to be included in a block.
/// Returns the channel ID that was created.
pub async fn setup_test_channel(executor: &Executor) -> ChannelId {
    let test_channel_id = ChannelId::from([1u8; 32]);
    let inscription_tx = create_inscription_transaction_with_id(test_channel_id);
    executor.add_tx(inscription_tx).await.unwrap();

    wait_for_inscription_onchain(executor, test_channel_id).await;

    test_channel_id
}

/// Creates an inscription transaction using the same hardcoded key as the mock
/// wallet adapter
#[must_use]
pub fn create_inscription_transaction_with_id(id: ChannelId) -> SignedMantleTx {
    let signing_key = SigningKey::from_bytes(&[0u8; 32]);
    let signer = signing_key.verifying_key();

    let inscription_op = InscriptionOp {
        channel_id: id,
        inscription: format!("Test channel inscription {id:?}").into_bytes(),
        parent: MsgId::root(),
        signer,
    };

    let mantle_tx = MantleTx {
        ops: vec![Op::ChannelInscribe(inscription_op)],
        ledger_tx: LedgerTx::new(vec![], vec![]),
        storage_gas_price: 0,
        execution_gas_price: 0,
    };

    let tx_hash = mantle_tx.hash();
    let signature = signing_key.sign(&tx_hash.as_signing_bytes());

    SignedMantleTx::new(
        mantle_tx,
        vec![OpProof::Ed25519Sig(signature)],
        DummyZkSignature::prove(&ZkSignaturePublic {
            msg_hash: tx_hash.into(),
            pks: vec![],
        }),
    )
    .unwrap()
}

/// Wait for inscription transaction to be included in a block
async fn wait_for_inscription_onchain(executor: &Executor, channel_id: ChannelId) {
    let block_fut = async {
        let mut onchain = false;
        while !onchain {
            let info = executor.consensus_info().await;
            if let Some(block) = executor.get_block(info.tip).await {
                onchain = block
                    .transactions()
                    .flat_map(|tx| tx.mantle_tx().ops.iter())
                    .any(|op| {
                        if let Op::ChannelInscribe(inscribe_op) = op {
                            inscribe_op.channel_id == channel_id
                        } else {
                            false
                        }
                    });
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };

    let timeout = adjust_timeout(Duration::from_secs(DA_TESTS_TIMEOUT));
    assert!(
        tokio::time::timeout(timeout, block_fut).await.is_ok(),
        "timed out waiting for inscription transaction to be included in block"
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
