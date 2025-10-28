use std::time::Duration;

use common_http_client::CommonHttpClient;
use ed25519_dalek::SigningKey;
use executor_http_client::ExecutorHttpClient;
use nomos_core::mantle::{
    Transaction as _,
    ops::channel::{ChannelId, Ed25519PublicKey, MsgId},
};
use serial_test::serial;
use tests::{
    common::{
        da::wait_for_blob_onchain,
        mantle::{
            create_channel_inscribe_tx, create_channel_set_keys_tx, fetch_channel_state,
            wait_for_channel_tip_change,
        },
    },
    topology::{Topology, TopologyConfig},
};

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[42u8; 32])
}

fn test_channel_id() -> ChannelId {
    ChannelId::from([1u8; 32])
}

#[tokio::test]
#[serial]
/// End-to-end channel lifecycle check covering:
/// 1. Inscribing an initial channel message and waiting for inclusion;
/// 2. Updating the channel key set with a fresh test key;
/// 3. Dispersing a blob via the executor to create a channel blob.
///
/// After every step the ledger state is verified
async fn channel_ops_flow() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let validator1 = &topology.validators()[0];
    let executor = &topology.executors()[0];
    let validator_url = validator1.url();
    let client = CommonHttpClient::new(None);

    topology.wait_network_ready().await;
    topology.wait_membership_ready().await;

    let signing_key = test_signing_key();
    let channel_id = test_channel_id();

    let signed_inscribe_tx = create_channel_inscribe_tx(
        &signing_key,
        channel_id,
        b"test message".to_vec(),
        MsgId::root(),
    );

    let inscribe_result = client
        .post_transaction(validator_url.clone(), signed_inscribe_tx.clone())
        .await;

    assert!(
        inscribe_result.is_ok(),
        "Failed to submit inscribe transaction"
    );

    let inscribe_hash = signed_inscribe_tx.hash();

    let inscribe_results = validator1
        .wait_for_transactions_inclusion(vec![inscribe_hash], Duration::from_secs(30))
        .await;

    assert!(
        inscribe_results[0].is_some(),
        "Inscribe transaction must be included in a block"
    );

    let state = wait_for_channel_tip_change(
        validator_url.as_str(),
        channel_id,
        MsgId::root(),
        Duration::from_secs(10),
    )
    .await
    .expect("Channel should exist after inscription");

    assert_ne!(
        state.tip,
        MsgId::root(),
        "Channel tip should not be root after inscription"
    );

    let new_signing_key = SigningKey::from_bytes(&[0u8; 32]);
    let new_authorized_key =
        Ed25519PublicKey::from_bytes(&new_signing_key.verifying_key().to_bytes())
            .expect("New verifying key must be valid");

    let set_keys_tx =
        create_channel_set_keys_tx(&signing_key, channel_id, vec![new_authorized_key]);

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

    let channel_state = fetch_channel_state(validator_url.as_str(), channel_id)
        .await
        .expect("Channel should exist after updating keys");

    assert_eq!(
        &*channel_state.keys,
        &[new_authorized_key],
        "Channel keys should be updated to the new key"
    );

    assert_ne!(
        channel_state.tip,
        MsgId::root(),
        "Channel tip should not be root before publishing blob"
    );

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
            new_authorized_key,
            blob_payload.to_vec(),
        )
        .await
        .expect("Failed to publish blob");

    wait_for_blob_onchain(executor, channel_id, blob_id).await;

    let final_channel_state = wait_for_channel_tip_change(
        validator_url.as_str(),
        channel_id,
        channel_state.tip,
        Duration::from_secs(10),
    )
    .await
    .expect("Channel tip should advance after blob dispersal");

    assert_ne!(
        final_channel_state.tip, channel_state.tip,
        "Channel tip should advance after blob dispersal"
    );
}
