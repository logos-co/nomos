use std::time::Duration;

use chain_service::CryptarchiaInfo;
use common_http_client::Error;
use ed25519_dalek::{Signer as _, SigningKey};
use executor_http_client::ExecutorHttpClient;
use futures::StreamExt as _;
use nomos_core::{
    da::BlobId,
    header::HeaderId,
    mantle::{
        AuthenticatedMantleTx as _, MantleTx, Op, OpProof, SignedMantleTx, Transaction as _,
        ledger::Tx as LedgerTx,
        ops::channel::{ChannelId, Ed25519PublicKey, MsgId, inscribe::InscriptionOp},
    },
};
use rand::rngs::OsRng;
use reqwest::Url;
use testing_framework_core::scenario::{ClientError, NodeClient};

use crate::{adjust_timeout, nodes::executor::Executor};

pub const DA_TESTS_TIMEOUT: u64 = 120;

#[must_use]
pub fn build_channel_inscription(channel_id: ChannelId) -> (SignedMantleTx, Ed25519PublicKey) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let signer = signing_key.verifying_key();

    let inscription_op = InscriptionOp {
        channel_id,
        inscription: format!("Test channel inscription {channel_id:?}").into_bytes(),
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

    let signed = SignedMantleTx::new(
        mantle_tx,
        vec![OpProof::Ed25519Sig(signature)],
        nomos_core::proofs::zksig::DummyZkSignature::prove(
            &nomos_core::proofs::zksig::ZkSignaturePublic {
                msg_hash: tx_hash.into(),
                pks: vec![],
            },
        ),
    )
    .expect("valid inscription transaction");

    (signed, signer)
}

pub async fn wait_for_channel_inclusion(
    client: &dyn NodeClient,
    channel_id: ChannelId,
    timeout: Duration,
) -> Result<(u64, HeaderId), ClientError> {
    let observe = async {
        loop {
            let info = client.consensus_info().await?;
            if let Some(block) = client.get_block(info.tip).await? {
                let found = block
                    .transactions()
                    .flat_map(|tx| tx.mantle_tx().ops.iter())
                    .any(|op| {
                        matches!(
                            op,
                            Op::ChannelInscribe(inscribe) if inscribe.channel_id == channel_id
                        )
                    });
                if found {
                    return Ok((info.height, info.tip));
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };

    tokio::time::timeout(timeout, observe)
        .await
        .map_err(|_| ClientError::Http("timed out waiting for channel inclusion".into()))?
}

pub async fn disseminate_with_metadata(executor: &Executor, data: &[u8]) -> Result<BlobId, Error> {
    let executor_config = executor.config();
    let backend_address = executor_config.http.backend_settings.address;
    let client = ExecutorHttpClient::new(None);
    let exec_url = Url::parse(&format!("http://{backend_address}")).unwrap();

    let channel_id = [0u8; 32].into();
    let parent_msg_id = MsgId::root();
    let signer = Ed25519PublicKey::from_bytes(&[1u8; 32]).unwrap();

    client
        .publish_blob(exec_url, channel_id, parent_msg_id, signer, data.to_vec())
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
