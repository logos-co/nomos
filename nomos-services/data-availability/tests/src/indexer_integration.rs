use std::{
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering::SeqCst},
        Arc,
    },
    time::Duration,
};

use cl::{NoteWitness, NullifierSecret};
use cryptarchia_consensus::{ConsensusMsg, LeaderConfig, TimeConfig};
use kzgrs_backend::{
    common::blob::DaBlob,
    dispersal::{BlobInfo, Metadata},
};
use nomos_core::{
    da::{
        blob::{metadata::Metadata as _, Blob},
        DaEncoder as _,
    },
    staking::NMO_UNIT,
};
use nomos_da_storage::rocksdb::{
    create_blob_idx, key_bytes, DA_BLOB_PREFIX, DA_SHARED_COMMITMENTS_PREFIX,
};
use nomos_da_verifier::backend::kzgrs::KzgrsDaVerifierSettings;
use nomos_ledger::LedgerState;
use nomos_libp2p::{Multiaddr, SwarmConfig};
use nomos_node::Wire;
use nomos_storage::{
    backends::{rocksdb::RocksBackend, StorageSerde},
    StorageMsg, StorageService,
};
use overwatch_rs::services::relay::OutboundRelay;
use rand::{thread_rng, Rng};
use tempfile::{NamedTempFile, TempDir};
use time::OffsetDateTime;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

use crate::common::*;

const INDEXER_TEST_MAX_SECONDS: u64 = 60;

// TODO: When verifier is implemented this test should be removed and a new one
// performed in integration tests crate using the real node.

#[test]
fn test_indexer() {
    let performed_tx = Arc::new(AtomicBool::new(false));
    let performed_rx = performed_tx.clone();
    let is_success_tx = Arc::new(AtomicBool::new(false));
    let is_success_rx = is_success_tx.clone();

    let mut ids = vec![[0; 32]; 2];
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

    let genesis_state = LedgerState::from_commitments(commitments, (ids.len() as u32).into());
    let ledger_config = nomos_ledger::Config {
        epoch_stake_distribution_stabilization: 3,
        epoch_period_nonce_buffer: 3,
        epoch_period_nonce_stabilization: 4,
        consensus_config: cryptarchia_engine::Config {
            security_param: 10,
            active_slot_coeff: 0.9,
        },
    };
    let time_config = TimeConfig {
        slot_duration: Duration::from_secs(1),
        chain_start_time: OffsetDateTime::now_utc(),
    };

    let swarm_config1 = SwarmConfig {
        port: 7771,
        ..Default::default()
    };
    let swarm_config2 = SwarmConfig {
        port: 7772,
        ..Default::default()
    };
    let blend_configs = new_blend_configs(vec![
        Multiaddr::from_str("/ip4/127.0.0.1/udp/7781/quic-v1").unwrap(),
        Multiaddr::from_str("/ip4/127.0.0.1/udp/7782/quic-v1").unwrap(),
    ]);

    let blobs_dir = TempDir::new().unwrap().path().to_path_buf();

    let (peer_sk_1, peer_id_1) = create_ed25519_sk_peerid(SK1);
    let (peer_sk_2, peer_id_2) = create_ed25519_sk_peerid(SK2);

    let addr_1 = Multiaddr::from_str("/ip4/127.0.0.1/udp/8780/quic-v1").unwrap();
    let addr_2 = Multiaddr::from_str("/ip4/127.0.0.1/udp/8781/quic-v1").unwrap();

    let peer_addresses = vec![(peer_id_1, addr_1.clone()), (peer_id_2, addr_2.clone())];

    let num_samples = 1;
    let num_subnets = 2;
    let nodes_per_subnet = 2;

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
            peer_addresses,
            listening_address: addr_2,
            num_subnets,
            num_samples,
            nodes_per_subnet,
            node_key: peer_sk_2,
        },
    );

    // Node1 relays.
    let node1_mempool = node1.handle().relay::<DaMempool>();
    let node1_storage = node1.handle().relay::<StorageService<RocksBackend<Wire>>>();
    let node1_indexer = node1.handle().relay::<DaIndexer>();
    let node1_consensus = node1.handle().relay::<Cryptarchia>();

    // Node2 relays.
    let node2_storage = node2.handle().relay::<StorageService<RocksBackend<Wire>>>();

    let encoder = &ENCODER;
    let data = rand_data(10);

    let encoded_data = encoder.encode(&data).unwrap();
    let columns: Vec<_> = encoded_data.extended_data.columns().collect();

    let mut blobs = vec![];
    for (i, column) in columns.iter().enumerate() {
        let da_blob = DaBlob {
            column: column.clone(),
            column_idx: i
                .try_into()
                .expect("Column index shouldn't overflow the target type"),
            column_commitment: encoded_data.column_commitments[i],
            aggregated_column_commitment: encoded_data.aggregated_column_commitment,
            aggregated_column_proof: encoded_data.aggregated_column_proofs[i],
            rows_commitments: encoded_data.row_commitments.clone(),
            rows_proofs: encoded_data
                .rows_proofs
                .iter()
                .map(|proofs| proofs.get(i).cloned().unwrap())
                .collect(),
        };

        blobs.push(da_blob);
    }

    let blob_0 = blobs[0].clone();
    let blobs_node_2 = blobs.clone();

    // Test generate hash for Certificate with default Hasher
    // let mut default_hasher = DefaultHasher::new();
    // let _hash3 = <BlobInfo as Hash>::hash(&blob_info, &mut default_hasher);

    let app_id = [7u8; 32];
    let index = 0.into();

    let range = 0.into()..1.into(); // get idx 0 and 1.
    let meta = Metadata::new(app_id, index);

    let blob_hash = <DaBlob as Blob>::id(&blobs[0]);
    let blob_info = BlobInfo::new(blob_hash, meta);

    // Create orphan blob metadata - without having actual blob stored
    let orphan_app_id = [10u8; 32];
    let orphan_index = 2.into();

    let orphan_meta = Metadata::new(orphan_app_id, orphan_index);
    let mut orphan_blob_hash = [0u8; 32];
    thread_rng().fill(&mut orphan_blob_hash[..]);

    let orphan_blob_info = BlobInfo::new(orphan_blob_hash, orphan_meta);

    // Test get Metadata for Certificate
    let (app_id2, index2) = blob_info.metadata();

    assert_eq!(app_id2, app_id);
    assert_eq!(index2, index);

    let expected_blob_info = blob_info.clone();

    node2.spawn(async move {
        let storage_outbound = node2_storage.connect().await.unwrap();
        store_blobs_in_db(blobs_node_2, storage_outbound).await;
    });

    node1.spawn(async move {
        let mempool_outbound = node1_mempool.connect().await.unwrap();
        let storage_outbound = node1_storage.connect().await.unwrap();
        let indexer_outbound = node1_indexer.connect().await.unwrap();
        let consensus_outbound = node1_consensus.connect().await.unwrap();

        let (sender, receiver) = tokio::sync::oneshot::channel();
        consensus_outbound
            .send(ConsensusMsg::BlockSubscribe { sender })
            .await
            .unwrap();
        let broadcast_receiver = receiver.await.unwrap();
        let mut broadcast_receiver =
            BroadcastStream::new(broadcast_receiver).filter_map(|result| result.ok());

        // Mock both attested blobs by writting directly into the da storage.
        store_blobs_in_db(blobs, storage_outbound).await;

        // Put blob_info into the mempool.
        let (mempool_tx, mempool_rx) = tokio::sync::oneshot::channel();
        mempool_outbound
            .send(nomos_mempool::MempoolMsg::Add {
                payload: blob_info,
                key: blob_hash,
                reply_channel: mempool_tx,
            })
            .await
            .unwrap();
        let _ = mempool_rx.await.unwrap();

        // Put orphan_blob_info into the mempool.
        let (mempool2_tx, mempool2_rx) = tokio::sync::oneshot::channel();
        mempool_outbound
            .send(nomos_mempool::MempoolMsg::Add {
                payload: orphan_blob_info,
                key: orphan_blob_hash,
                reply_channel: mempool2_tx,
            })
            .await
            .unwrap();
        let _ = mempool2_rx.await.unwrap();

        // Wait for block in the network.
        let timeout = tokio::time::sleep(Duration::from_secs(INDEXER_TEST_MAX_SECONDS));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                Some(block) = broadcast_receiver.next() => {
                    if block.blobs().any(|b| *b == expected_blob_info) {
                        break;
                    }
                }
                _ = &mut timeout => {
                    break;
                }
            }
        }

        // Give time for services to process and store data.
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Request range of blobs from indexer.
        let (indexer_tx, indexer_rx) = tokio::sync::oneshot::channel();
        indexer_outbound
            .send(nomos_da_indexer::DaMsg::GetRange {
                app_id,
                range,
                reply_channel: indexer_tx,
            })
            .await
            .unwrap();
        let mut app_id_blobs = indexer_rx.await.unwrap();

        // Request range of orphan blob from indexer - nothing is expected in return
        let (indexer2_tx, indexer2_rx) = tokio::sync::oneshot::channel();
        indexer_outbound
            .send(nomos_da_indexer::DaMsg::GetRange {
                app_id: orphan_app_id,
                range: 0.into()..2.into(),
                reply_channel: indexer2_tx,
            })
            .await
            .unwrap();
        let orphan_app_id_blobs = indexer2_rx.await.unwrap();

        // Indexer should not return any blobs for orphan app_id
        for v in orphan_app_id_blobs {
            assert!(v.1.is_empty());
        }

        // Mempool should still contain orphan_blob_info
        let (mempool3_tx, mempool3_rx) = tokio::sync::oneshot::channel();
        mempool_outbound
            .send(nomos_mempool::MempoolMsg::Status {
                items: vec![orphan_blob_hash],
                reply_channel: mempool3_tx,
            })
            .await
            .unwrap();
        let blocks_with_orphan_blob_hash = mempool3_rx.await.unwrap();

        assert_eq!(blocks_with_orphan_blob_hash.len(), 1);

        // Since we've only attested to blob_info at idx 0, the first
        // item should have "some" data, other indexes should be None.
        app_id_blobs.sort_by(|(a, _), (b, _)| a.partial_cmp(b).unwrap());
        let app_id_blobs = app_id_blobs.iter().map(|(_, b)| b).collect::<Vec<_>>();

        // When Indexer is asked for app_id at index, it will return all blobs that it
        // has for that blob_id.
        let columns = app_id_blobs[0];
        if !columns.is_empty() && columns[0] == blob_0 && app_id_blobs[1].is_empty() {
            is_success_tx.store(true, SeqCst);
        }

        performed_tx.store(true, SeqCst);
    });

    while !performed_rx.load(SeqCst) {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(is_success_rx.load(SeqCst));
}

async fn store_blobs_in_db(
    blobs: Vec<DaBlob>,
    storage_outbound: OutboundRelay<StorageMsg<RocksBackend<Wire>>>,
) {
    // Also all blobs in RocksDb
    for blob in blobs {
        let id = blob.id();
        let idx = create_blob_idx(id.as_ref(), blob.column_idx().as_ref());

        let (light_blob, shared_commitments) = blob.into_blob_and_shared_commitments();
        let blob_key = key_bytes(DA_BLOB_PREFIX, idx);
        storage_outbound
            .send(StorageMsg::Store {
                key: blob_key,
                value: Wire::serialize(light_blob),
            })
            .await
            .unwrap();

        let shared_commitments_key = key_bytes(DA_SHARED_COMMITMENTS_PREFIX, &id);
        let bytes = Wire::serialize(shared_commitments);
        storage_outbound
            .send(StorageMsg::Store {
                key: shared_commitments_key,
                value: bytes,
            })
            .await
            .unwrap();
    }
}
