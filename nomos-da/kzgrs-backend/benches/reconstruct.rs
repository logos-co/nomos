use std::{collections::HashSet, hint::black_box};

use ark_bls12_381::Fr;
use ark_poly::{EvaluationDomain, GeneralEvaluationDomain};
use divan::{counter::BytesCount, Bencher};
use kzgrs::{common::field_element_from_bytes_le, decode, rs::points_to_bytes};
use kzgrs_backend::{
    encoder::{DaEncoder, DaEncoderParams},
    global::GLOBAL_PARAMETERS,
};
use nomos_core::da::DaEncoder as _;
use rand::{prelude::IteratorRandom, thread_rng, RngCore};

fn main() {
    divan::main();
}

const MB: usize = 1024;

#[must_use] pub fn rand_data(elements_count: usize) -> Vec<u8> {
    let mut buff = vec![0; elements_count * DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE];
    rand::thread_rng().fill_bytes(&mut buff);
    buff
}
#[divan::bench(consts = [32, 64, 128, 256, 512, 1024], args = [128, 256, 512, 1024, 2048, 4096], sample_count = 1, sample_size = 1)]
fn reconstruct<const SIZE: usize>(bencher: Bencher, column_size: usize) {
    bencher
        .with_inputs(|| {
            let params = DaEncoderParams::new(column_size, true, GLOBAL_PARAMETERS.clone());
            let data = rand_data(SIZE * MB / DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE);
            let encoder = DaEncoder::new(params);
            encoder.encode(&data).unwrap()
        })
        .input_counter(|encoded| BytesCount::new(encoded.data.len()))
        .bench_values(|encoded| {
            let mut rng = thread_rng();
            black_box(move || {
                let idxs = (0..SIZE * 2)
                    .choose_multiple(&mut rng, SIZE)
                    .into_iter()
                    .collect::<HashSet<_>>();
                let rows = encoded.chunked_data.rows().map(|row| {
                    row.0
                        .iter()
                        .enumerate()
                        .map(|(i, chunk)| {
                            idxs.contains(&i)
                                .then(|| field_element_from_bytes_le(chunk.0.as_ref()))
                        })
                        .collect::<Vec<Option<Fr>>>()
                });
                let domain = GeneralEvaluationDomain::<Fr>::new(SIZE).unwrap();
                let data: Vec<u8> = rows
                    .map(|row| decode(SIZE, &row, domain))
                    .flat_map(|evals| points_to_bytes::<31>(&evals.evals))
                    .collect();
                assert_eq!(data, encoded.data);
            })
        });
}
