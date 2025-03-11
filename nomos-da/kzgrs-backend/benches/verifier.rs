use std::hint::black_box;

use divan::{counter::BytesCount, Bencher};
use kzgrs_backend::{
    common::{blob::DaBlob, Chunk},
    encoder::{DaEncoder, DaEncoderParams},
    global::GLOBAL_PARAMETERS,
    verifier::DaVerifier,
};
use nomos_core::da::{blob::Blob, DaEncoder as _};
use rand::{thread_rng, RngCore};

fn main() {
    divan::main();
}

const KB: usize = 1024;

#[must_use]
pub fn rand_data(elements_count: usize) -> Vec<u8> {
    let mut buff = vec![0; elements_count * DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE];
    thread_rng().fill_bytes(&mut buff);
    buff
}
#[divan::bench(consts = [32, 64, 128, 256, 512, 1024], args = [128, 256, 512, 1024, 2048, 4096], sample_count = 1, sample_size = 30)]
fn verify<const SIZE: usize>(bencher: Bencher, column_size: usize) {
    bencher
        .with_inputs(|| {
            let params = DaEncoderParams::new(column_size, true, GLOBAL_PARAMETERS.clone());

            let encoder = DaEncoder::new(params);
            let data = rand_data(SIZE * KB / DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE);
            let encoded_data = encoder.encode(&data).unwrap();
            let verifier = DaVerifier::new(GLOBAL_PARAMETERS.clone());
            let da_blob = DaBlob {
                column: encoded_data.extended_data.columns().next().unwrap(),
                column_idx: 0,
                column_commitment: encoded_data.column_commitments.first().copied().unwrap(),
                aggregated_column_commitment: encoded_data.aggregated_column_commitment,
                aggregated_column_proof: encoded_data
                    .aggregated_column_proofs
                    .first()
                    .copied()
                    .unwrap(),
                rows_commitments: encoded_data.row_commitments.clone(),
                rows_proofs: encoded_data
                    .rows_proofs
                    .iter()
                    .map(|row| row.iter().next().copied().unwrap())
                    .collect(),
            };
            (verifier, da_blob)
        })
        .input_counter(|(_, blob)| {
            BytesCount::new(blob.column.iter().map(Chunk::len).sum::<usize>())
        })
        .bench_values(|(verifier, blob)| {
            let (light_blob, commitments) = blob.into_blob_and_shared_commitments();
            black_box(verifier.verify(&commitments, &light_blob, column_size))
        });
}
