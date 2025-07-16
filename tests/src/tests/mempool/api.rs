use common_http_client::CommonHttpClient;
use nomos_core::mantle::{ledger::Tx as LedgerTx, MantleTx, SignedMantleTx};
use reqwest::Url;
use tests::topology::{Topology, TopologyConfig};

#[tokio::test]
async fn test_post_mantle_tx() {
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

    let mantle_tx = MantleTx {
        ops: Vec::new(),
        ledger_tx: LedgerTx::new(vec![], vec![]),
        storage_gas_price: 0,
        execution_gas_price: 0,
    };

    let signed_tx = SignedMantleTx {
        mantle_tx,
        ops_profs: Vec::new(),
        ledger_tx_proof: (),
    };

    let client = CommonHttpClient::new(None);
    let res = client.post_transaction(validator_url, signed_tx).await;
    assert!(res.is_ok());
}
