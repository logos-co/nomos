use groth16::{Field as _, Fr, Groth16Input, Groth16InputDeser};
use num_bigint::BigUint;
use serde::Serialize;

#[derive(Clone)]
pub struct PolPrivateInputs {
    secret_key: Groth16Input,
    note_value: Groth16Input,
    transaction_hash: Groth16Input,
    output_numer: Groth16Input,
    aged_proof: Groth16Input,
    aged_path: Vec<Groth16Input>,
    aged_selector: Vec<Groth16Input>,
    latest_proof: Groth16Input,
    latest_path: Vec<Groth16Input>,
    latest_selector: Vec<Groth16Input>,
    slot_secret: Groth16Input,
    secrets_root: Groth16Input,
    starting_slot: Groth16Input,
}

pub struct PolPrivateInputsData {
    pub secret_key: [u8; 32],
    pub note_value: u64,
    pub transaction_hash: [u8; 32],
    pub output_numer: u64,
    pub aged_proof: [u8; 32],
    pub aged_path: Vec<[u8; 32]>,
    pub aged_selector: Vec<bool>,
    pub latest_proof: [u8; 32],
    pub latest_path: Vec<[u8; 32]>,
    pub latest_selector: Vec<bool>,
    pub slot_secret: [u8; 32],
    pub secrets_root: [u8; 32],
    pub starting_slot: u64,
}

#[derive(Serialize)]
pub struct PolPrivateInputsJson {
    secret_key: Groth16InputDeser,
    note_value: Groth16InputDeser,
    transaction_hash: Groth16InputDeser,
    output_numer: Groth16InputDeser,
    aged_proof: Groth16InputDeser,
    aged_path: Vec<Groth16InputDeser>,
    aged_selector: Vec<Groth16InputDeser>,
    latest_proof: Groth16InputDeser,
    latest_path: Vec<Groth16InputDeser>,
    latest_selector: Vec<Groth16InputDeser>,
    slot_secret: Groth16InputDeser,
    secrets_root: Groth16InputDeser,
    starting_slot: Groth16InputDeser,
}
impl From<&PolPrivateInputs> for PolPrivateInputsJson {
    fn from(
        PolPrivateInputs {
            secret_key,
            note_value,
            transaction_hash,
            output_numer,
            aged_proof,
            aged_path,
            aged_selector,
            latest_proof,
            latest_path,
            latest_selector,
            slot_secret,
            secrets_root,
            starting_slot,
        }: &PolPrivateInputs,
    ) -> Self {
        Self {
            secret_key: secret_key.into(),
            note_value: note_value.into(),
            transaction_hash: transaction_hash.into(),
            output_numer: output_numer.into(),
            aged_proof: aged_proof.into(),
            aged_path: aged_path.iter().map(Into::into).collect(),
            aged_selector: aged_selector.iter().map(Into::into).collect(),
            latest_proof: latest_proof.into(),
            latest_path: latest_path.iter().map(Into::into).collect(),
            latest_selector: latest_selector.iter().map(Into::into).collect(),
            slot_secret: slot_secret.into(),
            secrets_root: secrets_root.into(),
            starting_slot: starting_slot.into(),
        }
    }
}

impl From<PolPrivateInputsData> for PolPrivateInputs {
    fn from(
        PolPrivateInputsData {
            secret_key,
            note_value,
            transaction_hash,
            output_numer,
            aged_proof,
            aged_path,
            aged_selector,
            latest_proof,
            latest_path,
            latest_selector,
            slot_secret,
            secrets_root,
            starting_slot,
        }: PolPrivateInputsData,
    ) -> Self {
        Self {
            secret_key: Groth16Input::new(Fr::from(BigUint::from_bytes_le(&secret_key))),
            note_value: Groth16Input::new(Fr::from(BigUint::from(note_value))),
            transaction_hash: Groth16Input::new(Fr::from(BigUint::from_bytes_le(
                &transaction_hash,
            ))),
            output_numer: Groth16Input::new(Fr::from(BigUint::from(output_numer))),
            aged_proof: Groth16Input::new(Fr::from(BigUint::from_bytes_le(&aged_proof))),
            aged_path: aged_path
                .into_iter()
                .map(|hash| Groth16Input::new(Fr::from(BigUint::from_bytes_le(&hash))))
                .collect(),
            aged_selector: aged_selector
                .into_iter()
                .map(|value: bool| Groth16Input::new(if value { Fr::ONE } else { Fr::ZERO }))
                .collect(),
            latest_proof: Groth16Input::new(Fr::from(BigUint::from_bytes_le(&latest_proof))),
            latest_path: latest_path
                .into_iter()
                .map(|hash| Groth16Input::new(Fr::from(BigUint::from_bytes_le(&hash))))
                .collect(),
            latest_selector: latest_selector
                .into_iter()
                .map(|value: bool| Groth16Input::new(if value { Fr::ONE } else { Fr::ZERO }))
                .collect(),
            slot_secret: Groth16Input::new(Fr::from(BigUint::from_bytes_le(&slot_secret))),
            secrets_root: Groth16Input::new(Fr::from(BigUint::from_bytes_le(&secrets_root))),
            starting_slot: Groth16Input::new(Fr::from(BigUint::from(starting_slot))),
        }
    }
}
