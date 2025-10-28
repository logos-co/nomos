use std::time::Duration;

use common_http_client::CommonHttpClient;
use executor_http_client::ExecutorHttpClient;
use nomos_core::mantle::{
    Transaction as _,
    ops::channel::{ChannelId, Ed25519PublicKey, MsgId},
};
use serial_test::serial;
use tests::{
    common::{
        da::wait_for_blob_onchain,
        mantle_tx::{create_channel_inscribe_tx, create_channel_set_keys_tx},
    },
    topology::{Topology, TopologyConfig},
};

fn test_signing_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[42u8; 32])
}

fn test_channel_id() -> ChannelId {
    ChannelId::from([1u8; 32])
}

async fn fetch_channel_state(
    base_url: &str,
    channel_id: ChannelId,
) -> Option<nomos_api::http::mantle_state::ChannelState> {
    let request_url = format!(
        "{}mantle/channels/{}",
        base_url,
        hex::encode(channel_id.as_ref()),
    );

    reqwest::get(&request_url)
        .await
        .unwrap()
        .json::<Option<nomos_api::http::mantle_state::ChannelState>>()
        .await
        .unwrap()
}

#[tokio::test]
#[serial]
async fn test_channel_operations_complete_e2e() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let validator1 = &topology.validators()[0];
    let executor = &topology.executors()[0];

    // Wait for nodes to initialise
    tokio::time::sleep(Duration::from_secs(5)).await;

    let signing_key = test_signing_key();
    let channel_id = test_channel_id();

    // Test 1: Submit CHANNEL_INSCRIBE transaction
    let signed_inscribe_tx = create_channel_inscribe_tx(
        &signing_key,
        channel_id,
        b"test message".to_vec(),
        MsgId::root(),
    );

    let validator_url = validator1.url();

    let client = CommonHttpClient::new(None);
    let inscribe_result = client
        .post_transaction(validator_url.clone(), signed_inscribe_tx.clone())
        .await;
    assert!(
        inscribe_result.is_ok(),
        "Failed to submit inscribe transaction"
    );

    let inscribe_hash = signed_inscribe_tx.hash();

    // Test 2: Wait for inscribe transaction inclusion
    let inscribe_results = validator1
        .wait_for_transactions_inclusion(vec![inscribe_hash], Duration::from_secs(30))
        .await;
    assert!(
        inscribe_results[0].is_some(),
        "Inscribe transaction must be included in a block"
    );

    // Ensure inscription has been processed and channel exists.
    let mut channel_ready = false;
    for _ in 0..10 {
        if fetch_channel_state(validator_url.as_str(), channel_id)
            .await
            .is_some_and(|state| state.tip != MsgId::root())
        {
            channel_ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(
        channel_ready,
        "Channel should exist after inscribe transaction"
    );

    // Update channel keys so that DA dispersal wallet is authorized to sign blob
    // messages.
    let wallet_signing_key = ed25519_dalek::SigningKey::from_bytes(&[0u8; 32]);
    let wallet_verifying_key =
        Ed25519PublicKey::from_bytes(&wallet_signing_key.verifying_key().to_bytes())
            .expect("Wallet verifying key must be valid");
    let set_keys_tx =
        create_channel_set_keys_tx(&signing_key, channel_id, vec![wallet_verifying_key]);

    let set_keys_hash = set_keys_tx.hash();
    let set_keys_result = client
        .post_transaction(validator_url.clone(), set_keys_tx)
        .await;
    assert!(
        set_keys_result.is_ok(),
        "Failed to submit set-keys transaction"
    );

    let set_keys_results = validator1
        .wait_for_transactions_inclusion(vec![set_keys_hash], Duration::from_secs(30))
        .await;
    assert!(
        set_keys_results[0].is_some(),
        "Set-keys transaction must be included in a block"
    );

    let mut channel_state = fetch_channel_state(validator_url.as_str(), channel_id)
        .await
        .expect("Channel should exist after updating keys");
    assert_eq!(
        channel_state.accredited_keys,
        vec![wallet_verifying_key],
        "Channel keys should be updated to wallet key"
    );

    // Ensure we observe the non-root tip before submitting a blob transaction.
    for _ in 0..10 {
        if channel_state.tip != MsgId::root() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        channel_state = fetch_channel_state(validator_url.as_str(), channel_id)
            .await
            .expect("Channel should exist after key update");
    }
    assert!(
        channel_state.tip != MsgId::root(),
        "Channel tip should not be root before publishing blob"
    );

    // Test 4: Publish blob via executor service to create a legitimate CHANNEL_BLOB
    // transaction.
    let blob_payload = [1u8; 31];
    let executor_config = executor.config();
    let backend_address = executor_config.http.backend_settings.address;
    let exec_url =
        reqwest::Url::parse(&format!("http://{backend_address}")).expect("Valid executor URL");

    let blob_id = ExecutorHttpClient::new(None)
        .publish_blob(
            exec_url,
            channel_id,
            channel_state.tip,
            wallet_verifying_key,
            blob_payload.to_vec(),
        )
        .await
        .expect("Failed to publish blob");
    wait_for_blob_onchain(executor, blob_id).await;

    // Test 5: Check if ledger state was actually updated by blob submission
    let final_channel_state = fetch_channel_state(validator_url.as_str(), channel_id)
        .await
        .expect("Channel should exist after blob dispersal");
    assert!(
        final_channel_state
            .accredited_keys
            .contains(&wallet_verifying_key),
        "Channel keys should still include wallet key after blob dispersal"
    );
}
