use ark_ff::{BigInteger as _, Field as _, PrimeField as _};
use groth16::{serde::serde_fr, Fr};
use num_bigint::BigUint;
use poseidon2::{Digest as _, Poseidon2Bn254Hasher};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const POL_PROOF_DEV_MODE: &str = "POL_PROOF_DEV_MODE";

use crate::{
    mantle::{ledger::Utxo, ops::leader_claim::VoucherCm},
    utils::merkle::{MerkleNode, MerklePath},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Groth16LeaderProof {
    #[serde(with = "proof_serde")]
    proof: pol::PoLProof,
    #[serde(with = "serde_fr")]
    entropy_contribution: Fr,
    leader_key: ed25519_dalek::VerifyingKey,
    voucher_cm: VoucherCm,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Proof of leadership failed: {0}")]
    PoLProofFailed(#[from] pol::ProveError),
}

impl Groth16LeaderProof {
    pub fn prove(private: &LeaderPrivate, voucher_cm: VoucherCm) -> Result<Self, Error> {
        let start_t = std::time::Instant::now();
        let (proof, entropy_contribution) = if std::env::var(POL_PROOF_DEV_MODE).is_ok() {
            tracing::warn!(
                "Proofs are being generated in dev mode. This should never be used in production."
            );
            (
                groth16::CompressedGroth16Proof::new(
                    [0; 32].into(),
                    [0; 64].into(),
                    [0; 32].into(),
                ),
                Fr::ZERO,
            )
        } else {
            let (proof, verif_inputs) =
                pol::prove(&private.input).map_err(Error::PoLProofFailed)?;
            (proof, verif_inputs.entropy_contribution.into_inner())
        };
        tracing::debug!("groth16 prover time: {:.2?}", start_t.elapsed(),);

        let leader_key = private.pk;

        Ok(Self {
            proof,
            entropy_contribution,
            leader_key,
            voucher_cm,
        })
    }

    #[must_use]
    pub const fn proof(&self) -> &pol::PoLProof {
        &self.proof
    }
}

pub trait LeaderProof {
    /// Verify the proof against the public inputs.
    fn verify(&self, public_inputs: &LeaderPublic) -> bool;

    /// Get the entropy used in the proof.
    fn entropy(&self) -> Fr;

    fn leader_key(&self) -> &ed25519_dalek::VerifyingKey;

    fn voucher_cm(&self) -> &VoucherCm;
}

impl LeaderProof for Groth16LeaderProof {
    fn verify(&self, public_inputs: &LeaderPublic) -> bool {
        if std::env::var(POL_PROOF_DEV_MODE).is_ok() {
            tracing::warn!(
                "Proofs are being verified in dev mode. This should never be used in production."
            );
            return true;
        }
        let leader_pk_bytes = self.leader_key.as_bytes();
        let leader_pk = (
            Fr::from(u128::from_le_bytes(
                leader_pk_bytes[0..16].try_into().unwrap(),
            )),
            Fr::from(u128::from_le_bytes(
                leader_pk_bytes[16..32].try_into().unwrap(),
            )),
        );
        pol::verify(
            &self.proof,
            &pol::PolVerifierInput::new(
                self.entropy(),
                public_inputs.slot,
                public_inputs.epoch_nonce,
                public_inputs.aged_root,
                public_inputs.latest_root,
                public_inputs.total_stake,
                leader_pk,
            ),
        )
        .is_ok()
    }

    fn entropy(&self) -> Fr {
        self.entropy_contribution
    }

    fn leader_key(&self) -> &ed25519_dalek::VerifyingKey {
        &self.leader_key
    }

    fn voucher_cm(&self) -> &VoucherCm {
        &self.voucher_cm
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderPublic {
    pub slot: u64,
    #[serde(with = "serde_fr")]
    pub epoch_nonce: Fr,
    pub total_stake: u64,
    #[serde(with = "serde_fr")]
    pub aged_root: Fr,
    #[serde(with = "serde_fr")]
    pub latest_root: Fr,
}

impl LeaderPublic {
    #[must_use]
    pub const fn new(
        aged_root: Fr,
        latest_root: Fr,
        epoch_nonce: Fr,
        slot: u64,
        total_stake: u64,
    ) -> Self {
        Self {
            slot,
            epoch_nonce,
            total_stake,
            aged_root,
            latest_root,
        }
    }

    #[must_use]
    pub fn check_winning(&self, value: u64, note_id: Fr, sk: Fr) -> bool {
        let threshold = Self::phi_approx(&BigUint::from(value), &self.scaled_phi_approx());
        let threshold_bytes = threshold.to_bytes_le();
        let threshold = BigUint::from_bytes_le(&threshold_bytes);
        let ticket = Self::ticket(note_id, sk, self.epoch_nonce, Fr::from(self.slot));
        let ticket_bytes = ticket.into_bigint().to_bytes_le();
        let ticket_bigint = BigUint::from_bytes_le(&ticket_bytes);
        ticket_bigint < threshold
    }

    fn scaled_phi_approx(&self) -> (BigUint, BigUint) {
        let t0 = &*pol::T0_CONSTANT / &BigUint::from(self.total_stake);
        let total_stake_sq = &BigUint::from(self.total_stake) * &BigUint::from(self.total_stake);
        let t1 = &*pol::P - (&*pol::T1_CONSTANT / &total_stake_sq);
        (t0, t1)
    }

    fn phi_approx(stake: &BigUint, approx: &(BigUint, BigUint)) -> BigUint {
        // stake * (t0 - t1 * stake)
        stake * (&approx.0 - (&approx.1 * stake))
    }

    fn ticket(note_id: Fr, sk: Fr, epoch_nonce: Fr, slot: Fr) -> Fr {
        Poseidon2Bn254Hasher::digest(&[note_id, sk, epoch_nonce, slot])
    }
}

#[derive(Debug, Clone)]
pub struct LeaderPrivate {
    input: pol::PolWitnessInputs,
    pk: ed25519_dalek::VerifyingKey,
}

impl LeaderPrivate {
    #[must_use]
    pub fn new(
        public: LeaderPublic,
        note: Utxo,
        aged_path: &MerklePath<Fr>,
        latest_path: &MerklePath<Fr>,
        slot_secret: Fr,
        starting_slot: u64,
        leader_pk: &ed25519_dalek::VerifyingKey,
    ) -> Self {
        let public_key = *leader_pk;
        let (pk_1, pk_2) = (
            Fr::from(u128::from_le_bytes(
                public_key.as_bytes()[0..16].try_into().unwrap(),
            )),
            Fr::from(u128::from_le_bytes(
                public_key.as_bytes()[16..32].try_into().unwrap(),
            )),
        );
        let chain = pol::PolChainInputsData {
            slot_number: public.slot,
            epoch_nonce: public.epoch_nonce,
            total_stake: public.total_stake,
            aged_root: public.aged_root,
            latest_root: public.latest_root,
            leader_pk: (pk_1, pk_2),
        };
        let wallet = pol::PolWalletInputsData {
            note_value: note.note.value,
            transaction_hash: *note.tx_hash.as_ref(),
            output_number: note.output_index as u64,
            aged_path: aged_path.iter().map(|n| *n.item()).collect(),
            aged_selector: aged_path
                .iter()
                .map(|n| matches!(n, MerkleNode::Right(_)))
                .collect(),
            latest_path: latest_path.iter().map(|n| *n.item()).collect(),
            latest_selector: latest_path
                .iter()
                .map(|n| matches!(n, MerkleNode::Right(_)))
                .collect(),
            slot_secret,
            slot_secret_path: vec![], // TODO: implement
            starting_slot,
        };
        let input = pol::PolWitnessInputs::from_chain_and_wallet_data(chain, wallet);
        Self {
            input,
            pk: public_key,
        }
    }
}

mod proof_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(item: &pol::PoLProof, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&item.to_bytes())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<pol::PoLProof, D::Error>
    where
        D: Deserializer<'de>,
    {
        let proof_bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
        let proof_array: [u8; 128] = proof_bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("Expected exactly 128 bytes"))?;
        Ok(pol::PoLProof::from_bytes(&proof_array))
    }
}
