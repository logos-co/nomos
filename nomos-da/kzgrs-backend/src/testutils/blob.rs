use nomos_core::da::DaEncoder as _;

use crate::{common::share::DaShare, testutils::encoder::get_encoder};

#[must_use]
pub fn get_default_da_blob_data() -> Vec<u8> {
    vec![
        49u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
        0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
    ]
}

pub fn get_da_share(data: Option<Vec<u8>>) -> DaShare {
    let encoder = get_encoder();

    let data = data.unwrap_or_else(get_default_da_blob_data);
    let encoded_data = encoder.encode(&data).unwrap();
    let columns: Vec<_> = encoded_data.extended_data.columns().collect();

    let index = 0;
    let da_share = DaShare {
        column: columns[index].clone(),
        share_idx: index
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

    da_share
}
