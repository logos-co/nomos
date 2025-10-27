use std::{num::NonZero, sync::Arc};

use chain_leader::LeaderConfig;
use cryptarchia_engine::EpochConfig;
use ed25519_dalek::ed25519::signature::SignerMut as _;
use nomos_core::{
    mantle::{
        MantleTx, Note, OpProof, Utxo,
        genesis_tx::GenesisTx,
        keys::{PublicKey, SecretKey},
        ledger::Tx as LedgerTx,
        ops::{
            Op,
            channel::{ChannelId, Ed25519PublicKey, MsgId, inscribe::InscriptionOp},
        },
    },
    proofs::zksig::{DummyZkSignature, ZkSignaturePublic},
    sdp::{DeclarationMessage, Locator, ProviderId, ServiceParameters, ServiceType, ZkPublicKey},
};
use nomos_node::Transaction as _;
use num_bigint::BigUint;

#[derive(Clone)]
pub struct ConsensusParams {
    pub n_participants: usize,
    pub security_param: NonZero<u32>,
    pub active_slot_coeff: f64,
}

impl ConsensusParams {
    #[must_use]
    pub const fn default_for_participants(n_participants: usize) -> Self {
        Self {
            n_participants,
            // by setting the slot coeff to 1, we also increase the probability of multiple blocks
            // (forks) being produced in the same slot (epoch). Setting the security
            // parameter to some value > 1 ensures nodes have some time to sync before
            // deciding on the longest chain.
            security_param: NonZero::new(10).unwrap(),
            // a block should be produced (on average) every slot
            active_slot_coeff: 0.9,
        }
    }
}

#[derive(Clone)]
pub struct ProviderInfo {
    pub service_type: ServiceType,
    pub provider_id: ProviderId,
    pub zk_id: ZkPublicKey,
    pub locator: Locator,
    pub note: Note,
    pub signer: ed25519_dalek::SigningKey,
}

/// General consensus configuration for a chosen participant, that later could
/// be converted into a specific service or services configuration.
#[derive(Clone)]
pub struct GeneralConsensusConfig {
    pub leader_config: LeaderConfig,
    pub ledger_config: nomos_ledger::Config,
    pub genesis_tx: GenesisTx,
    pub utxos: Vec<Utxo>,
    pub blend_notes: Vec<ServiceNote>,
    pub da_notes: Vec<ServiceNote>,
}

#[derive(Clone)]
pub struct ServiceNote {
    pub pk: PublicKey,
    pub sk: SecretKey,
    pub note: Note,
}

fn create_genesis_tx(utxos: &[Utxo]) -> GenesisTx {
    // Create a genesis inscription op (similar to config.yaml)
    let inscription = InscriptionOp {
        channel_id: ChannelId::from([0; 32]),
        inscription: vec![103, 101, 110, 101, 115, 105, 115], // "genesis" in bytes
        parent: MsgId::root(),
        signer: Ed25519PublicKey::from_bytes(&[0; 32]).unwrap(),
    };

    // Create ledger transaction with the utxos as outputs
    let outputs: Vec<Note> = utxos.iter().map(|u| u.note).collect();
    let ledger_tx = LedgerTx::new(vec![], outputs);

    // Create the mantle transaction
    let mantle_tx = MantleTx {
        ops: vec![Op::ChannelInscribe(inscription)],
        ledger_tx,
        execution_gas_price: 0,
        storage_gas_price: 0,
    };

    // Wrap in GenesisTx
    GenesisTx::from_tx(mantle_tx)
        .expect("Invalid genesis transaction")
        .with_proofs(vec![None])
        .expect("One empty proof for inscription")
}

#[must_use]
pub fn create_consensus_configs(
    ids: &[[u8; 32]],
    consensus_params: &ConsensusParams,
) -> Vec<GeneralConsensusConfig> {
    let derive_key_material = |prefix: &[u8], id_bytes: &[u8]| -> [u8; 16] {
        let mut sk_data = [0; 16];
        let prefix_len = prefix.len();

        sk_data[..prefix_len].copy_from_slice(prefix);
        let remaining_len = 16 - prefix_len;
        sk_data[prefix_len..].copy_from_slice(&id_bytes[..remaining_len]);

        sk_data
    };

    let mut leader_keys = Vec::new();
    let mut da_notes = Vec::new();
    let mut blend_notes = Vec::new();
    let mut utxos = Vec::new();

    // Create notes for leader, Blend and DA declarations.
    for &id in ids.iter() {
        let sk_leader_data = derive_key_material(b"ld", &id);
        let sk_leader = SecretKey::from(BigUint::from_bytes_le(&sk_leader_data));
        let pk_leader = sk_leader.to_public_key();
        leader_keys.push((pk_leader, sk_leader));
        utxos.push(Utxo {
            note: Note::new(1, pk_leader),
            tx_hash: BigUint::from(0u8).into(),
            output_index: 0,
        });

        let sk_da_data = derive_key_material(b"da", &id);
        let sk_da = SecretKey::from(BigUint::from_bytes_le(&sk_da_data));
        let pk_da = sk_da.to_public_key();
        let note_da = Note::new(1, pk_da);
        da_notes.push(ServiceNote {
            pk: pk_da,
            sk: sk_da,
            note: note_da,
        });
        utxos.push(Utxo {
            note: note_da,
            tx_hash: BigUint::from(0u8).into(),
            output_index: 0,
        });

        let sk_blend_data = derive_key_material(b"bn", &id);
        let sk_blend = SecretKey::from(BigUint::from_bytes_le(&sk_blend_data));
        let pk_blend = sk_blend.to_public_key();
        let note_blend = Note::new(1, pk_blend);
        blend_notes.push(ServiceNote {
            pk: pk_blend,
            sk: sk_blend,
            note: note_blend,
        });
        utxos.push(Utxo {
            note: note_blend,
            tx_hash: BigUint::from(0u8).into(),
            output_index: 0,
        });
    }

    let genesis_tx = create_genesis_tx(&utxos);
    let ledger_config = nomos_ledger::Config {
        epoch_config: EpochConfig {
            epoch_stake_distribution_stabilization: NonZero::new(3).unwrap(),
            epoch_period_nonce_buffer: NonZero::new(3).unwrap(),
            epoch_period_nonce_stabilization: NonZero::new(4).unwrap(),
        },
        consensus_config: cryptarchia_engine::Config {
            security_param: consensus_params.security_param,
            active_slot_coeff: consensus_params.active_slot_coeff,
        },
        sdp_config: nomos_ledger::mantle::sdp::Config {
            service_params: Arc::new(
                [
                    (
                        ServiceType::BlendNetwork,
                        ServiceParameters {
                            lock_period: 10,
                            inactivity_period: 20,
                            retention_period: 100,
                            timestamp: 0,
                            session_duration: 1000,
                        },
                    ),
                    (
                        ServiceType::DataAvailability,
                        ServiceParameters {
                            lock_period: 10,
                            inactivity_period: 20,
                            retention_period: 100,
                            timestamp: 0,
                            session_duration: 1000,
                        },
                    ),
                ]
                .into(),
            ),
            min_stake: nomos_core::sdp::MinStake {
                threshold: 1,
                timestamp: 0,
            },
        },
    };

    leader_keys
        .into_iter()
        .map(|(pk, sk)| GeneralConsensusConfig {
            leader_config: LeaderConfig { pk, sk },
            ledger_config: ledger_config.clone(),
            genesis_tx: genesis_tx.clone(),
            utxos: utxos.clone(),
            da_notes: da_notes.clone(),
            blend_notes: blend_notes.clone(),
        })
        .collect()
}

#[must_use]
pub fn create_genesis_tx_with_declarations(
    ledger_tx: LedgerTx,
    providers: Vec<ProviderInfo>,
) -> GenesisTx {
    let inscription = InscriptionOp {
        channel_id: ChannelId::from([0; 32]),
        inscription: vec![103, 101, 110, 101, 115, 105, 115], // "genesis" in bytes
        parent: MsgId::root(),
        signer: Ed25519PublicKey::from_bytes(&[0; 32]).unwrap(),
    };
    let ledger_tx_hash = ledger_tx.hash();

    let mut ops = vec![Op::ChannelInscribe(inscription)];

    for provider in &providers {
        let utxo = Utxo {
            tx_hash: ledger_tx_hash,
            output_index: 0,
            note: provider.note,
        };
        let declaration = DeclarationMessage {
            service_type: provider.service_type,
            locators: vec![provider.locator.clone()],
            provider_id: ProviderId(provider.signer.verifying_key()),
            zk_id: provider.zk_id,
            locked_note_id: utxo.id(),
        };
        ops.push(Op::SDPDeclare(declaration));
    }

    let mantle_tx = MantleTx {
        ops,
        ledger_tx,
        execution_gas_price: 0,
        storage_gas_price: 0,
    };

    let mantle_tx_hash = mantle_tx.hash();
    let mut op_proofs = vec![None];

    for mut provider in providers {
        let zk_sig = DummyZkSignature::prove(&ZkSignaturePublic {
            pks: vec![provider.note.pk.into(), provider.zk_id.0],
            msg_hash: mantle_tx_hash.0,
        });
        let ed25519_sig = provider
            .signer
            .sign(mantle_tx_hash.as_signing_bytes().as_ref());

        op_proofs.push(Some(OpProof::ZkAndEd25519Sigs {
            zk_sig,
            ed25519_sig,
        }));
    }

    GenesisTx::from_tx(mantle_tx)
        .expect("Invalid genesis transaction")
        .with_proofs(op_proofs)
        .expect("Correct number of proofs should be set")
}
