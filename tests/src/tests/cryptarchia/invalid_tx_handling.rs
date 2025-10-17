use std::time::Duration;

use common_http_client::CommonHttpClient;
use nomos_core::{
    mantle::{
        MantleTx, Note, SignedMantleTx, Transaction as _, TxHash, keys::PublicKey,
        ledger::Tx as LedgerTx,
    },
    proofs::zksig::{DummyZkSignature, ZkSignaturePublic},
};
use num_bigint::BigUint;
use reqwest::Url;
use serial_test::serial;
use tests::{
    common::da::create_inscription_transaction_with_id,
    nodes::validator::Validator,
    topology::{Topology, TopologyConfig},
};

const EXTRA_TXS: usize = 5;
const TOTAL_TXS: usize = 100;

#[tokio::test]
#[serial]
async fn test_leader_skips_invalid_transactions() {
    let topology = Topology::spawn(TopologyConfig::validator_and_executor()).await;
    let validator = &topology.validators()[0];

    let validator_url = Url::parse(
        format!(
            "http://{}",
            validator.config().http.backend_settings.address
        )
        .as_str(),
    )
    .unwrap();

    let client = CommonHttpClient::new(None);

    let mut valid_tx_hashes = Vec::new();
    let mut invalid_tx_hashes = Vec::new();

    println!("Sending {TOTAL_TXS} transactions (extra={EXTRA_TXS})");

    for i in 0..EXTRA_TXS {
        let invalid_tx = create_invalid_transaction_with_id(i);
        let invalid_hash = invalid_tx.hash();
        let result = client
            .post_transaction(validator_url.clone(), invalid_tx)
            .await;

        if let Err(err) = &result {
            println!("Invalid transaction {invalid_hash:?} rejected immediately: {err:?}");
        }

        invalid_tx_hashes.push(invalid_hash);
    }

    for i in 0..TOTAL_TXS {
        // Most transactions are valid
        let valid_tx = create_inscription_transaction_with_id(i);
        valid_tx_hashes.push(valid_tx.hash());

        let result = client
            .post_transaction(validator_url.clone(), valid_tx)
            .await;
        assert!(
            result.is_ok(),
            "Transaction {i} should be accepted by mempool: {result:?}"
        );
    }

    println!(
        "All {TOTAL_TXS} transactions posted. Valid: {}, Invalid: {}",
        valid_tx_hashes.len(),
        invalid_tx_hashes.len()
    );

    tokio::time::timeout(
        Duration::from_secs(60),
        wait_for_bulk_transaction_processing(validator, &valid_tx_hashes, &invalid_tx_hashes),
    )
    .await
    .expect("transaction processing timed out");
}

/// Wait for bulk transaction processing and verify results
async fn wait_for_bulk_transaction_processing(
    validator: &Validator,
    valid_tx_hashes: &[TxHash],
    invalid_tx_hashes: &[TxHash],
) {
    let mut found_valid_txs = std::collections::HashSet::new();
    let mut found_invalid_txs = std::collections::HashSet::new();
    let mut processed_blocks = std::collections::HashSet::new();
    // Genesis parent (all zeros) does not have an accessible block; mark as
    // processed.
    processed_blocks.insert([0u8; 32].into());

    // Wait until we find all valid transactions
    loop {
        let info = validator.consensus_info().await;
        let mut stack = vec![info.tip];
        let mut processed_any = false;
        let mut need_retry = false;

        while let Some(block_id) = stack.pop() {
            if !processed_blocks.insert(block_id) {
                continue;
            }

            if let Some(block) = validator.get_block(block_id).await {
                processed_any = true;

                let block_tx_hashes: Vec<TxHash> = block
                    .transactions()
                    .map(nomos_core::mantle::Transaction::hash)
                    .collect();

                // Collect valid and invalid transactions found in this block
                for &valid_hash in valid_tx_hashes {
                    if block_tx_hashes.contains(&valid_hash) {
                        found_valid_txs.insert(valid_hash);
                    }
                }

                for &invalid_hash in invalid_tx_hashes {
                    if block_tx_hashes.contains(&invalid_hash) {
                        found_invalid_txs.insert(invalid_hash);
                    }
                }

                // If we've seen all valid transactions, we can stop immediately.
                if found_valid_txs.len() == valid_tx_hashes.len() {
                    break;
                }

                // Traverse parent chain to make sure intermediate blocks are inspected
                let parent = block.header().parent();
                stack.push(parent);
            } else {
                // Block not yet available; retry on next iteration.
                processed_blocks.remove(&block_id);
                need_retry = true;
                break;
            }
        }

        if found_valid_txs.len() == valid_tx_hashes.len() {
            break;
        }

        println!(
            "Found {}/{} valid transactions so far...",
            found_valid_txs.len(),
            valid_tx_hashes.len()
        );

        if !processed_any || need_retry {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    assert_eq!(
        valid_tx_hashes.len(),
        found_valid_txs.len(),
        "Not all valid transactions were included in blocks"
    );

    // Verify no invalid transactions made it into blocks
    assert!(
        found_invalid_txs.is_empty(),
        "Invalid transactions found in blocks: {found_invalid_txs:?}"
    );
}

fn create_invalid_transaction_with_id(id: usize) -> SignedMantleTx {
    let output_note = Note::new(1000 + id as u64, PublicKey::new(BigUint::from(1u8).into()));

    let mantle_tx = MantleTx {
        ops: Vec::new(),
        ledger_tx: LedgerTx::new(vec![], vec![output_note]),
        storage_gas_price: 0,
        execution_gas_price: 0,
    };

    SignedMantleTx {
        ops_proofs: Vec::new(),
        ledger_tx_proof: DummyZkSignature::prove(ZkSignaturePublic {
            msg_hash: mantle_tx.hash().into(),
            pks: vec![],
        }),
        mantle_tx,
    }
}
