use lb_groth16::{AdditiveGroup as _, Field as _, Fr, Groth16Input, Groth16InputDeser};
// TODO(RFC-480): depth 32 UTXO tree witness
// use lb_pol::AGED_NOTE_MERKLE_TREE_HEIGHT;
use lb_pol::NOTE_SET_TREE_DEPTH;
use num_bigint::BigUint;
use serde::Serialize;

// TODO(RFC-480): depth 32 UTXO tree witness
// pub type AgedNotePathAndSelectors = [(Fr, bool); AGED_NOTE_MERKLE_TREE_HEIGHT];

// note commitment in its set MMR, from the leaf level up
pub type AgedNotePathAndSelectors = [(Fr, bool); NOTE_SET_TREE_DEPTH];
// set leaf in the eligible tree, from the leaf level up
pub type AgedSetPathAndSelectors = [(Fr, bool); NOTE_SET_TREE_DEPTH];

#[derive(Clone)]
pub struct PoQWalletInputs {
    pol_slot: Groth16Input,
    pol_secret_key: Groth16Input,
    // TODO(RFC-480): depth 32 UTXO tree witness
    // pol_noteid_path_and_selectors: [(Groth16Input, Groth16Input); AGED_NOTE_MERKLE_TREE_HEIGHT],
    // pol_note_tx_hash: Groth16Input,
    // pol_note_output_number: Groth16Input,
    pol_note_value: Groth16Input,
    pol_note_nonce: Groth16Input,
    pol_cm_aged_path_and_selectors: [(Groth16Input, Groth16Input); NOTE_SET_TREE_DEPTH],
    pol_nf_root_aged: Groth16Input,
    pol_set_aged_path_and_selectors: [(Groth16Input, Groth16Input); NOTE_SET_TREE_DEPTH],
}

pub struct PoQWalletInputsData {
    pub slot: u64,
    pub note_value: u64,
    // TODO(RFC-480): depth 32 UTXO tree witness
    // pub transaction_hash: Fr,
    // pub output_number: u64,
    // pub aged_path_and_selectors: AgedNotePathAndSelectors,
    pub note_nonce: Fr,
    pub cm_aged_path_and_selectors: AgedNotePathAndSelectors,
    pub nf_root_aged: Fr,
    pub set_aged_path_and_selectors: AgedSetPathAndSelectors,
    pub pol_secret_key: Fr,
}

#[derive(Serialize)]
pub struct PoQWalletInputsJson {
    #[serde(rename = "pol_sl")]
    pol_slot: Groth16InputDeser,
    #[serde(rename = "pol_note_value")]
    note_value: Groth16InputDeser,
    // TODO(RFC-480): depth 32 UTXO tree witness
    // #[serde(rename = "pol_note_tx_hash")]
    // transaction_hash: Groth16InputDeser,
    // #[serde(rename = "pol_note_output_number")]
    // output_number: Groth16InputDeser,
    // #[serde(rename = "pol_noteid_path")]
    // aged_path: [Groth16InputDeser; AGED_NOTE_MERKLE_TREE_HEIGHT],
    // #[serde(rename = "pol_noteid_path_selectors")]
    // aged_selector: [Groth16InputDeser; AGED_NOTE_MERKLE_TREE_HEIGHT],
    // serde serializes arrays only up to length 32
    #[serde(rename = "pol_note_nonce")]
    note_nonce: Groth16InputDeser,
    #[serde(rename = "pol_cm_aged_path")]
    cm_aged_path: Vec<Groth16InputDeser>,
    #[serde(rename = "pol_cm_aged_selectors")]
    cm_aged_selectors: Vec<Groth16InputDeser>,
    #[serde(rename = "pol_nf_root_aged")]
    nf_root_aged: Groth16InputDeser,
    #[serde(rename = "pol_set_aged_path")]
    set_aged_path: Vec<Groth16InputDeser>,
    #[serde(rename = "pol_set_selectors")]
    set_selectors: Vec<Groth16InputDeser>,
    #[serde(rename = "pol_secret_key")]
    pol_secret_key: Groth16InputDeser,
}
impl From<PoQWalletInputs> for PoQWalletInputsJson {
    fn from(
        PoQWalletInputs {
            pol_slot,
            pol_secret_key,
            // TODO(RFC-480): depth 32 UTXO tree witness
            // pol_noteid_path_and_selectors,
            // pol_note_tx_hash,
            // pol_note_output_number,
            pol_note_value,
            pol_note_nonce,
            pol_cm_aged_path_and_selectors,
            pol_nf_root_aged,
            pol_set_aged_path_and_selectors,
        }: PoQWalletInputs,
    ) -> Self {
        // TODO(RFC-480): depth 32 UTXO tree witness
        // let (aged_path, aged_selector) = {
        //     let aged_path = pol_noteid_path_and_selectors.map(|(path, _)| (&path).into());
        //     let aged_selector =
        //         pol_noteid_path_and_selectors.map(|(_, selector)| (&selector).into());
        //     (aged_path, aged_selector)
        // };

        Self {
            pol_slot: (&pol_slot).into(),
            note_value: (&pol_note_value).into(),
            // TODO(RFC-480): depth 32 UTXO tree witness
            // transaction_hash: (&pol_note_tx_hash).into(),
            // output_number: (&pol_note_output_number).into(),
            // aged_path,
            // aged_selector,
            note_nonce: (&pol_note_nonce).into(),
            cm_aged_path: pol_cm_aged_path_and_selectors
                .iter()
                .map(|(path, _)| path.into())
                .collect(),
            cm_aged_selectors: pol_cm_aged_path_and_selectors
                .iter()
                .map(|(_, selector)| selector.into())
                .collect(),
            nf_root_aged: (&pol_nf_root_aged).into(),
            set_aged_path: pol_set_aged_path_and_selectors
                .iter()
                .map(|(path, _)| path.into())
                .collect(),
            set_selectors: pol_set_aged_path_and_selectors
                .iter()
                .map(|(_, selector)| selector.into())
                .collect(),
            pol_secret_key: (&pol_secret_key).into(),
        }
    }
}

impl From<PoQWalletInputsData> for PoQWalletInputs {
    fn from(
        PoQWalletInputsData {
            slot,
            note_value,
            // TODO(RFC-480): depth 32 UTXO tree witness
            // transaction_hash,
            // output_number,
            // aged_path_and_selectors,
            note_nonce,
            cm_aged_path_and_selectors,
            nf_root_aged,
            set_aged_path_and_selectors,
            pol_secret_key,
        }: PoQWalletInputsData,
    ) -> Self {
        Self {
            pol_slot: Groth16Input::new(Fr::from(BigUint::from(slot))),
            pol_secret_key: pol_secret_key.into(),
            // TODO(RFC-480): depth 32 UTXO tree witness
            // pol_noteid_path_and_selectors: aged_path_and_selectors.map(|(hash, selector)| {
            //     (
            //         hash.into(),
            //         Groth16Input::new(if selector { Fr::ONE } else { Fr::ZERO }),
            //     )
            // }),
            // pol_note_tx_hash: transaction_hash.into(),
            // pol_note_output_number: Groth16Input::new(Fr::from(BigUint::from(output_number))),
            pol_note_value: Groth16Input::new(Fr::from(BigUint::from(note_value))),
            pol_note_nonce: note_nonce.into(),
            pol_cm_aged_path_and_selectors: cm_aged_path_and_selectors.map(|(hash, selector)| {
                (
                    hash.into(),
                    Groth16Input::new(if selector { Fr::ONE } else { Fr::ZERO }),
                )
            }),
            pol_nf_root_aged: nf_root_aged.into(),
            pol_set_aged_path_and_selectors: set_aged_path_and_selectors.map(|(hash, selector)| {
                (
                    hash.into(),
                    Groth16Input::new(if selector { Fr::ONE } else { Fr::ZERO }),
                )
            }),
        }
    }
}
