use std::{
    num::NonZero,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering::SeqCst},
        Arc,
    },
    time::Duration,
};

use cl::{NoteWitness, NullifierSecret};
use cryptarchia_consensus::LeaderConfig;
use cryptarchia_engine::{time::SlotConfig, EpochConfig};
use kzgrs_backend::common::blob::DaBlob;
use nomos_core::{da::DaEncoder as _, staking::NMO_UNIT};
use nomos_da_verifier::backend::kzgrs::KzgrsDaVerifierSettings;
use nomos_ledger::LedgerState;
use nomos_libp2p::{Multiaddr, SwarmConfig};
use rand::{thread_rng, Rng};
use tempfile::{NamedTempFile, TempDir};
use time::OffsetDateTime;
use tracing_subscriber::{fmt::TestWriter, EnvFilter};

use crate::common::*;

#[test]
#[expect(clippy::too_many_lines)]
fn test_verifier() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .compact()
        .with_writer(TestWriter::default())
        .try_init();
    let performed_tx = Arc::new(AtomicBool::new(false));
    let performed_rx = performed_tx.clone();
    let is_success_tx = Arc::new(AtomicBool::new(false));
    let is_success_rx = is_success_tx.clone();

    let mut ids = vec![[0; 32]; 3];
    for id in &mut ids {
        thread_rng().fill(id);
    }
    let sks = ids
        .iter()
        .map(|&id| {
            let mut sk = [0; 16];
            sk.copy_from_slice(&id[0..16]);
            NullifierSecret(sk)
        })
        .collect::<Vec<_>>();

    let notes = (0..ids.len())
        .map(|_| NoteWitness::basic(1, NMO_UNIT, &mut thread_rng()))
        .collect::<Vec<_>>();

    let commitments = notes.iter().zip(&sks).map(|(n, sk)| n.commit(sk.commit()));
    let genesis_state = LedgerState::from_commitments(commitments, ids.len().try_into().unwrap());
    let ledger_config = nomos_ledger::Config {
        epoch_config: EpochConfig {
            epoch_stake_distribution_stabilization: NonZero::new(3).unwrap(),
            epoch_period_nonce_buffer: NonZero::new(3).unwrap(),
            epoch_period_nonce_stabilization: NonZero::new(4).unwrap(),
        },
        consensus_config: cryptarchia_engine::Config {
            security_param: NonZero::new(10).unwrap(),
            active_slot_coeff: 0.9,
        },
    };
    let time_config = SlotConfig {
        slot_duration: Duration::from_secs(1),
        chain_start_time: OffsetDateTime::now_utc(),
    };

    let swarm_config1 = SwarmConfig {
        port: 7773,
        ..Default::default()
    };
    let swarm_config2 = SwarmConfig {
        port: 7774,
        ..Default::default()
    };

    let swarm_config3 = SwarmConfig {
        port: 7775,
        ..Default::default()
    };

    let blend_configs = new_blend_configs(&[
        Multiaddr::from_str("/ip4/127.0.0.1/udp/7783/quic-v1").unwrap(),
        Multiaddr::from_str("/ip4/127.0.0.1/udp/7784/quic-v1").unwrap(),
        Multiaddr::from_str("/ip4/127.0.0.1/udp/7785/quic-v1").unwrap(),
    ]);

    let blobs_dir = TempDir::new().unwrap().path().to_path_buf();

    let client_zone = new_client(NamedTempFile::new().unwrap().path().to_path_buf());

    let (peer_sk_1, peer_id_1) = generate_ed25519_sk_peerid();
    let (peer_sk_2, peer_id_2) = generate_ed25519_sk_peerid();
    let (peer_sk_3, peer_id_3) = generate_ed25519_sk_peerid();

    let addr_1 = Multiaddr::from_str("/ip4/127.0.0.1/udp/8880/quic-v1").unwrap();
    let addr_2 = Multiaddr::from_str("/ip4/127.0.0.1/udp/8881/quic-v1").unwrap();
    let addr_3 = Multiaddr::from_str("/ip4/127.0.0.1/udp/8882/quic-v1").unwrap();

    let peer_addresses = vec![
        (peer_id_1, addr_1.clone()),
        (peer_id_2, addr_2.clone()),
        (peer_id_3, addr_3.clone()),
    ];

    let num_samples = 1;
    let num_subnets = 3;
    let nodes_per_subnet = 1;

    let node1 = new_node(
        &LeaderConfig {
            notes: vec![notes[0]],
            nf_sk: sks[0],
        },
        &ledger_config,
        &genesis_state,
        &time_config,
        &swarm_config1,
        &blend_configs[0],
        NamedTempFile::new().unwrap().path().to_path_buf(),
        &blobs_dir,
        vec![node_address(&swarm_config2)],
        KzgrsDaVerifierSettings {
            global_params_path: GLOBAL_PARAMS_PATH.into(),
        },
        TestDaNetworkSettings {
            peer_addresses: peer_addresses.clone(),
            listening_address: addr_1,
            num_subnets,
            num_samples,
            nodes_per_subnet,
            node_key: peer_sk_1,
        },
    );

    let node2 = new_node(
        &LeaderConfig {
            notes: vec![notes[1]],
            nf_sk: sks[1],
        },
        &ledger_config,
        &genesis_state,
        &time_config,
        &swarm_config2,
        &blend_configs[1],
        NamedTempFile::new().unwrap().path().to_path_buf(),
        &blobs_dir,
        vec![node_address(&swarm_config1)],
        KzgrsDaVerifierSettings {
            global_params_path: GLOBAL_PARAMS_PATH.into(),
        },
        TestDaNetworkSettings {
            peer_addresses: peer_addresses.clone(),
            listening_address: addr_2,
            num_subnets,
            num_samples,
            nodes_per_subnet,
            node_key: peer_sk_2,
        },
    );

    let node3 = new_node(
        &LeaderConfig {
            notes: vec![notes[2]],
            nf_sk: sks[2],
        },
        &ledger_config,
        &genesis_state,
        &time_config,
        &swarm_config3,
        &blend_configs[2],
        NamedTempFile::new().unwrap().path().to_path_buf(),
        &blobs_dir,
        vec![node_address(&swarm_config2)],
        KzgrsDaVerifierSettings {
            global_params_path: GLOBAL_PARAMS_PATH.into(),
        },
        TestDaNetworkSettings {
            peer_addresses,
            listening_address: addr_3,
            num_subnets,
            num_samples,
            nodes_per_subnet,
            node_key: peer_sk_3,
        },
    );

    let node1_verifier = node1.handle().relay::<DaVerifier>();
    let node2_verifier = node2.handle().relay::<DaVerifier>();
    let node3_verifier = node3.handle().relay::<DaVerifier>();

    client_zone.spawn(async move {
        let node1_verifier = node1_verifier.connect().await.unwrap();
        let (node1_reply_tx, node1_reply_rx) = tokio::sync::oneshot::channel();

        let node2_verifier = node2_verifier.connect().await.unwrap();
        let (node2_reply_tx, node2_reply_rx) = tokio::sync::oneshot::channel();

        let node3_verifier = node3_verifier.connect().await.unwrap();
        let (node3_reply_tx, node3_reply_rx) = tokio::sync::oneshot::channel::<Option<()>>();

        let verifiers = vec![
            (node1_verifier, node1_reply_tx),
            (node2_verifier, node2_reply_tx),
            (node3_verifier, node3_reply_tx),
        ];

        // Encode data
        let encoder = &ENCODER;
        let data = vec![
            49u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
            0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
        ];

        let encoded_data = encoder.encode(&data).unwrap();
        let columns: Vec<_> = encoded_data.extended_data.columns().collect();

        drop(node3_reply_rx);

        for (i, (verifier, reply_tx)) in verifiers.into_iter().enumerate() {
            let index = i % 2;
            let column = &columns[index];

            let da_blob = DaBlob {
                column: column.clone(),
                column_idx: index
                    .try_into()
                    .expect("Column index shouldn't overflow the target type"),
                column_commitment: encoded_data.column_commitments[index],
                aggregated_column_commitment: encoded_data.aggregated_column_commitment,
                aggregated_column_proof: encoded_data.aggregated_column_proofs[index],
                rows_commitments: encoded_data.row_commitments.clone(),
                rows_proofs: encoded_data
                    .rows_proofs
                    .iter()
                    .map(|proofs| proofs.get(index).copied().unwrap())
                    .collect(),
            };

            let add_blob_message = nomos_da_verifier::DaVerifierMsg::AddBlob {
                blob: da_blob,
                reply_channel: reply_tx,
            };

            verifier.send(add_blob_message).await.unwrap();
        }

        // Wait for response from the verifier.
        let a1 = node1_reply_rx.await.unwrap();
        let a2 = node2_reply_rx.await.unwrap();

        if a1.is_some() && a2.is_some() {
            is_success_tx.store(true, SeqCst);
        }

        performed_tx.store(true, SeqCst);
    });

    while !performed_rx.load(SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(is_success_rx.load(SeqCst));
}
