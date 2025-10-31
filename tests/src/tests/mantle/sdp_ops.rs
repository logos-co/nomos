use std::{collections::HashSet, time::Duration};

use chain_service::StartingState;
use common_http_client::CommonHttpClient;
use ed25519_dalek::SigningKey;
use groth16::Fr;
use nomos_core::{
    mantle::{GenesisTx as _, Note, NoteId, Transaction as _, keys::PublicKey},
    sdp::{ActiveMessage, Declaration, Locator, ServiceType, SessionNumber, WithdrawMessage},
};
use serial_test::serial;
use tests::{
    adjust_timeout,
    common::mantle_tx::{
        create_sdp_active_tx, create_sdp_declare_tx, create_sdp_withdraw_tx,
        empty_da_activity_proof,
    },
    nodes::validator::Validator,
    topology::{Topology, TopologyConfig},
};
use tokio::time::{sleep, timeout};

/// High-level SDP flow covered by this E2E:
/// - submit a `Declare` transaction backed by an unused genesis note and wait
///   for inclusion;
/// - activate the declaration and poll the REST test endpoint until the
///   `Active` height reflects the update;
/// - advance past the lock period,`Withdraw`, and verify the declaration
///   disappears.
#[tokio::test]
#[serial]
async fn sdp_ops_e2e() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    topology.wait_network_ready().await;
    topology
        .wait_membership_ready_for_session(SessionNumber::from(0u64))
        .await;
    let validator = &topology.validators()[0];

    wait_for_height(validator, 1, adjust_timeout(Duration::from_secs(30)))
        .await
        .expect("validator should produce the first block before submitting declare");

    let inclusion_timeout = Duration::from_secs(30);
    let state_timeout = Duration::from_secs(45);

    let sdp_config = &validator.config().cryptarchia.config.sdp_config;

    let validator_url = validator.url();
    let client = CommonHttpClient::new(None);

    let existing = validator.get_sdp_declarations().await;
    let locked: HashSet<_> = existing.iter().map(|decl| decl.locked_note_id).collect();

    let (note, locked_note_id) = unused_genesis_note(validator.config(), &locked)
        .expect("Topology should expose an unused genesis note");
    let note_value = note.value;
    let note_public_key = note.pk;

    let zk_id = PublicKey::new(Fr::from(2u64));
    let provider_signing_key = SigningKey::from_bytes(&[7u8; 32]);
    let locator = Locator(
        "/ip4/127.0.0.1/tcp/9100"
            .parse()
            .expect("Valid locator multiaddr"),
    );

    let (declare_tx, declaration_msg) = create_sdp_declare_tx(
        &provider_signing_key,
        ServiceType::DataAvailability,
        vec![locator],
        zk_id,
        locked_note_id,
        Note::new(note_value, note_public_key),
    );
    let declaration_id = declaration_msg.id();
    let declare_hash = declare_tx.hash();

    client
        .post_transaction(validator_url.clone(), declare_tx)
        .await
        .expect("submit declare transaction");

    let declare_results = validator
        .wait_for_transactions_inclusion(vec![declare_hash], inclusion_timeout)
        .await;

    assert!(
        declare_results.first().is_some_and(Option::is_some),
        "declare transaction should be included"
    );

    let declaration_state = wait_for_declaration(validator, state_timeout, {
        let target_locked_note = locked_note_id;
        move |decl| decl.locked_note_id == target_locked_note
    })
    .await
    .expect("declaration should appear after submission");

    let lock_period = sdp_config
        .service_params
        .get(&ServiceType::DataAvailability)
        .expect("data availability parameters must exist")
        .lock_period;
    let height_timeout =
        adjust_timeout(Duration::from_secs(lock_period.saturating_mul(12).max(90)));

    let created_height = declaration_state.created;
    let initial_active = declaration_state.active;
    let mut current_nonce = declaration_state.nonce;

    let active_message = ActiveMessage {
        declaration_id,
        nonce: current_nonce + 1,
        metadata: empty_da_activity_proof(),
    };

    let active_tx = create_sdp_active_tx(
        active_message,
        zk_id,
        Note::new(note_value, note_public_key),
    );

    let active_hash = active_tx.hash();

    client
        .post_transaction(validator_url.clone(), active_tx)
        .await
        .expect("submit active transaction");

    let active_results = validator
        .wait_for_transactions_inclusion(vec![active_hash], inclusion_timeout)
        .await;

    assert!(
        active_results.first().is_some_and(Option::is_some),
        "active transaction should be included"
    );

    current_nonce += 1;

    wait_for_height(validator, created_height + 1, state_timeout)
        .await
        .expect("chain should advance after the active transaction");

    wait_for_declaration(validator, state_timeout, {
        move |decl| decl.locked_note_id == locked_note_id && decl.active > initial_active
    })
    .await
    .expect("Declaration state did not update after active transaction");

    wait_for_height(validator, created_height + lock_period + 1, height_timeout)
        .await
        .expect("consensus height should pass the SDP lock period");

    let withdraw_message = WithdrawMessage {
        declaration_id,
        locked_note_id,
        nonce: current_nonce + 1,
    };

    let withdraw_tx = create_sdp_withdraw_tx(
        withdraw_message,
        zk_id,
        Note::new(note_value, note_public_key),
    );
    let withdraw_hash = withdraw_tx.hash();

    client
        .post_transaction(validator_url, withdraw_tx)
        .await
        .expect("submit withdraw transaction");

    let withdraw_results = validator
        .wait_for_transactions_inclusion(vec![withdraw_hash], inclusion_timeout)
        .await;

    assert!(
        withdraw_results.first().is_some_and(Option::is_some),
        "withdraw transaction should be included"
    );

    let removed = wait_for_declaration_absence(validator, locked_note_id, state_timeout).await;
    assert!(removed, "withdraw should remove the declaration");
}

async fn wait_for_declaration<F>(
    validator: &Validator,
    duration: Duration,
    predicate: F,
) -> Option<Declaration>
where
    F: Fn(&Declaration) -> bool + Send + Sync + 'static,
{
    timeout(duration, async {
        loop {
            let declarations = validator.get_sdp_declarations().await;
            if let Some(declaration) = declarations.into_iter().find(|decl| predicate(decl)) {
                break declaration;
            }

            sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .ok()
}

async fn wait_for_declaration_absence(
    validator: &Validator,
    locked_note_id: NoteId,
    duration: Duration,
) -> bool {
    timeout(duration, async {
        loop {
            let present = validator
                .get_sdp_declarations()
                .await
                .into_iter()
                .any(|decl| decl.locked_note_id == locked_note_id);

            if !present {
                break;
            }

            sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .is_ok()
}

async fn wait_for_height(
    validator: &Validator,
    target_height: u64,
    duration: Duration,
) -> Option<()> {
    timeout(duration, async {
        loop {
            let info = validator.consensus_info().await;
            if info.height >= target_height {
                break;
            }

            sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .ok()
}

fn unused_genesis_note(
    config: &nomos_node::Config,
    locked: &HashSet<NoteId>,
) -> Option<(Note, NoteId)> {
    let genesis_tx = match &config.cryptarchia.starting_state {
        StartingState::Genesis { genesis_tx } => genesis_tx,
        StartingState::Lib { .. } => panic!("Topology test config should start from genesis"),
    };

    genesis_tx
        .mantle_tx()
        .ledger_tx
        .utxos()
        .find(|utxo| !locked.contains(&utxo.id()))
        .map(|utxo| (utxo.note, utxo.id()))
}
