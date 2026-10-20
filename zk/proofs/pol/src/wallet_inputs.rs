use lb_groth16::{AdditiveGroup as _, Field as _, Fr, Groth16Input, Groth16InputDeser};
use num_bigint::BigUint;
use serde::Serialize;

// TODO: These consts should live in a common crate shared among circuits.
// TODO(RFC-480): replaced by NOTE_SET_TREE_DEPTH
// pub const AGED_NOTE_MERKLE_TREE_HEIGHT: usize = 32;
// TODO(RFC-480): depth 32 UTXO tree witness
// pub type AgedNotePath = [Fr; AGED_NOTE_MERKLE_TREE_HEIGHT];
// pub type AgedSelectorPath = [bool; AGED_NOTE_MERKLE_TREE_HEIGHT];
// pub const LATEST_NOTE_MERKLE_TREE_HEIGHT: usize = 32;
// pub type LatestNotePath = [Fr; LATEST_NOTE_MERKLE_TREE_HEIGHT];
// pub type LatestSelectorPath = [bool; LATEST_NOTE_MERKLE_TREE_HEIGHT];
// Depth of the commitment MMR, the nullifier IMT and the eligible tree.
pub const NOTE_SET_TREE_DEPTH: usize = 40;

/// Public inputs of the POL cirmcom circuit as circuit field values.
#[derive(Clone, Debug)]
pub struct PolWalletInputs {
    note_value: Groth16Input,
    // TODO(RFC-480): depth 32 UTXO tree witness
    // transaction_hash: Groth16Input,
    // output_number: Groth16Input,
    // aged_path: [Groth16Input; AGED_NOTE_MERKLE_TREE_HEIGHT], // leaf-to-root
    // aged_selectors: [Groth16Input; AGED_NOTE_MERKLE_TREE_HEIGHT], // root-to-leaf
    // latest_path: [Groth16Input; LATEST_NOTE_MERKLE_TREE_HEIGHT], // leaf-to-root
    // latest_selectors: [Groth16Input; LATEST_NOTE_MERKLE_TREE_HEIGHT], // root-to-leaf
    // every path and selector list from the leaf level up
    note_nonce: Groth16Input,
    set_selectors: [Groth16Input; NOTE_SET_TREE_DEPTH],
    cm_aged_path: [Groth16Input; NOTE_SET_TREE_DEPTH],
    cm_aged_selectors: [Groth16Input; NOTE_SET_TREE_DEPTH],
    nf_root_aged: Groth16Input,
    set_aged_path: [Groth16Input; NOTE_SET_TREE_DEPTH],
    low_leaf_nf: Groth16Input,
    low_leaf_next_nf: Groth16Input,
    low_leaf_next_index: Groth16Input,
    low_leaf_path: [Groth16Input; NOTE_SET_TREE_DEPTH],
    low_leaf_selectors: [Groth16Input; NOTE_SET_TREE_DEPTH],
    cm_root_latest: Groth16Input,
    set_latest_path: [Groth16Input; NOTE_SET_TREE_DEPTH],
    secret_key: Groth16Input,
}

/// Private inputs of the POL cirmcom circuit to be provided by the wallet.
#[derive(Clone, Debug)]
pub struct PolWalletInputsData {
    pub note_value: u64,
    // TODO(RFC-480): depth 32 UTXO tree witness
    // pub transaction_hash: Fr,
    // pub output_number: u64,
    // pub aged_path: AgedNotePath,              // leaf-to-root
    // pub aged_selectors: AgedSelectorPath,     // root-to-leaf
    // pub latest_path: LatestNotePath,          // leaf-to-root
    // pub latest_selectors: LatestSelectorPath, // root-to-leaf
    // every path and selector list from the leaf level up
    pub note_nonce: Fr,
    pub set_selectors: [bool; NOTE_SET_TREE_DEPTH],
    pub cm_aged_path: [Fr; NOTE_SET_TREE_DEPTH],
    pub cm_aged_selectors: [bool; NOTE_SET_TREE_DEPTH],
    pub nf_root_aged: Fr,
    pub set_aged_path: [Fr; NOTE_SET_TREE_DEPTH],
    pub low_leaf_nf: Fr,
    pub low_leaf_next_nf: Fr,
    pub low_leaf_next_index: u64,
    pub low_leaf_path: [Fr; NOTE_SET_TREE_DEPTH],
    pub low_leaf_selectors: [bool; NOTE_SET_TREE_DEPTH],
    pub cm_root_latest: Fr,
    pub set_latest_path: [Fr; NOTE_SET_TREE_DEPTH],
    pub secret_key: Fr,
}

#[derive(Serialize)]
pub struct PolWalletInputsJson {
    // TODO(RFC-480): depth 32 UTXO tree witness
    // #[serde(rename = "v")]
    #[serde(rename = "value")]
    note_value: Groth16InputDeser,
    // TODO(RFC-480): depth 32 UTXO tree witness
    // #[serde(rename = "note_tx_hash")]
    // transaction_hash: Groth16InputDeser,
    // #[serde(rename = "note_output_number")]
    // output_number: Groth16InputDeser,
    // #[serde(rename = "noteid_aged_path")]
    // aged_path: [Groth16InputDeser; AGED_NOTE_MERKLE_TREE_HEIGHT], // leaf-to-root
    // #[serde(rename = "noteid_aged_selectors")]
    // aged_selectors: [Groth16InputDeser; AGED_NOTE_MERKLE_TREE_HEIGHT], // root-to-leaf
    // #[serde(rename = "noteid_latest_path")]
    // latest_path: [Groth16InputDeser; LATEST_NOTE_MERKLE_TREE_HEIGHT], // leaf-to-root
    // #[serde(rename = "noteid_latest_selectors")]
    // latest_selectors: [Groth16InputDeser; LATEST_NOTE_MERKLE_TREE_HEIGHT], // root-to-leaf
    // serde serializes arrays only up to length 32
    #[serde(rename = "nonce")]
    note_nonce: Groth16InputDeser,
    set_selectors: Vec<Groth16InputDeser>,
    cm_aged_path: Vec<Groth16InputDeser>,
    cm_aged_selectors: Vec<Groth16InputDeser>,
    nf_root_aged: Groth16InputDeser,
    set_aged_path: Vec<Groth16InputDeser>,
    low_leaf_nf: Groth16InputDeser,
    low_leaf_next_nf: Groth16InputDeser,
    low_leaf_next_index: Groth16InputDeser,
    low_leaf_path: Vec<Groth16InputDeser>,
    low_leaf_selectors: Vec<Groth16InputDeser>,
    cm_root_latest: Groth16InputDeser,
    set_latest_path: Vec<Groth16InputDeser>,
    #[serde(rename = "sk")]
    secret_key: Groth16InputDeser,
}
impl From<&PolWalletInputs> for PolWalletInputsJson {
    fn from(
        PolWalletInputs {
            note_value,
            // TODO(RFC-480): depth 32 UTXO tree witness
            // transaction_hash,
            // output_number,
            // aged_path,
            // aged_selectors,
            // latest_path,
            // latest_selectors,
            note_nonce,
            set_selectors,
            cm_aged_path,
            cm_aged_selectors,
            nf_root_aged,
            set_aged_path,
            low_leaf_nf,
            low_leaf_next_nf,
            low_leaf_next_index,
            low_leaf_path,
            low_leaf_selectors,
            cm_root_latest,
            set_latest_path,
            secret_key,
        }: &PolWalletInputs,
    ) -> Self {
        Self {
            note_value: note_value.into(),
            // TODO(RFC-480): depth 32 UTXO tree witness
            // transaction_hash: transaction_hash.into(),
            // output_number: output_number.into(),
            // aged_path: aged_path.map(|path| (&path).into()),
            // aged_selectors: aged_selectors.map(|selector| (&selector).into()),
            // latest_path: latest_path.map(|path| (&path).into()),
            // latest_selectors: latest_selectors.map(|selector| (&selector).into()),
            note_nonce: note_nonce.into(),
            set_selectors: set_selectors.iter().map(Into::into).collect(),
            cm_aged_path: cm_aged_path.iter().map(Into::into).collect(),
            cm_aged_selectors: cm_aged_selectors.iter().map(Into::into).collect(),
            nf_root_aged: nf_root_aged.into(),
            set_aged_path: set_aged_path.iter().map(Into::into).collect(),
            low_leaf_nf: low_leaf_nf.into(),
            low_leaf_next_nf: low_leaf_next_nf.into(),
            low_leaf_next_index: low_leaf_next_index.into(),
            low_leaf_path: low_leaf_path.iter().map(Into::into).collect(),
            low_leaf_selectors: low_leaf_selectors.iter().map(Into::into).collect(),
            cm_root_latest: cm_root_latest.into(),
            set_latest_path: set_latest_path.iter().map(Into::into).collect(),
            secret_key: secret_key.into(),
        }
    }
}

impl From<PolWalletInputsData> for PolWalletInputs {
    fn from(
        PolWalletInputsData {
            note_value,
            // TODO(RFC-480): depth 32 UTXO tree witness
            // transaction_hash,
            // output_number,
            // aged_path,
            // aged_selectors,
            // latest_path,
            // latest_selectors,
            note_nonce,
            set_selectors,
            cm_aged_path,
            cm_aged_selectors,
            nf_root_aged,
            set_aged_path,
            low_leaf_nf,
            low_leaf_next_nf,
            low_leaf_next_index,
            low_leaf_path,
            low_leaf_selectors,
            cm_root_latest,
            set_latest_path,
            secret_key,
        }: PolWalletInputsData,
    ) -> Self {
        Self {
            note_value: Groth16Input::new(Fr::from(BigUint::from(note_value))),
            // TODO(RFC-480): depth 32 UTXO tree witness
            // transaction_hash: transaction_hash.into(),
            // output_number: Groth16Input::new(Fr::from(BigUint::from(output_number))),
            // aged_path: aged_path.map(Into::into),
            // aged_selectors: aged_selectors
            //     .map(|selector| Groth16Input::new(if selector { Fr::ONE } else { Fr::ZERO })),
            // latest_path: latest_path.map(Into::into),
            // latest_selectors: latest_selectors
            //     .map(|selector| Groth16Input::new(if selector { Fr::ONE } else { Fr::ZERO })),
            note_nonce: note_nonce.into(),
            set_selectors: set_selectors
                .map(|selector| Groth16Input::new(if selector { Fr::ONE } else { Fr::ZERO })),
            cm_aged_path: cm_aged_path.map(Into::into),
            cm_aged_selectors: cm_aged_selectors
                .map(|selector| Groth16Input::new(if selector { Fr::ONE } else { Fr::ZERO })),
            nf_root_aged: nf_root_aged.into(),
            set_aged_path: set_aged_path.map(Into::into),
            low_leaf_nf: low_leaf_nf.into(),
            low_leaf_next_nf: low_leaf_next_nf.into(),
            low_leaf_next_index: Groth16Input::new(Fr::from(BigUint::from(low_leaf_next_index))),
            low_leaf_path: low_leaf_path.map(Into::into),
            low_leaf_selectors: low_leaf_selectors
                .map(|selector| Groth16Input::new(if selector { Fr::ONE } else { Fr::ZERO })),
            cm_root_latest: cm_root_latest.into(),
            set_latest_path: set_latest_path.map(Into::into),
            secret_key: secret_key.into(),
        }
    }
}
