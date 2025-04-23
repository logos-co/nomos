use std::hint::black_box;

use divan::{counter::BytesCount, Bencher};
use kzgrs_backend::encoder::DaEncoder2;
use kzgrs_backend::{
    encoder::{DaEncoder, DaEncoderParams},
    global::GLOBAL_PARAMETERS,
};
use rand::RngCore;

fn main() {
    divan::main();
}

const MB: usize = 1024;

#[must_use]
pub fn rand_data(elements_count: usize) -> Vec<u8> {
    let mut buff = vec![0; elements_count * DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE];
    rand::thread_rng().fill_bytes(&mut buff);
    buff
}

#[divan::bench(consts = [32, 64, 128, 256, 512, 1024], args = [128, 256, 512, 1024, 2048, 4096], sample_count = 1, sample_size = 1)]
fn encode<const SIZE: usize>(bencher: Bencher, column_size: usize) {
    bencher
        .with_inputs(|| {
            let params = DaEncoderParams::new(column_size, true, GLOBAL_PARAMETERS.clone());
            (
                DaEncoder::new(params),
                rand_data(SIZE * MB / DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE),
            )
        })
        .input_counter(|(_, buff)| BytesCount::new(buff.len()))
        .bench_refs(|(encoder, buff)| black_box(nomos_core::da::DaEncoder::encode(encoder, buff)));
}

#[divan::bench(consts = [32, 64, 128, 256, 512, 1024], args = [128, 256, 512, 1024, 2048, 4096], sample_count = 1, sample_size = 1)]
fn encode_bdfg<const SIZE: usize>(bencher: Bencher, column_size: usize) {
    bencher
        .with_inputs(|| {
            let params = DaEncoderParams::new(column_size, true, GLOBAL_PARAMETERS.clone());
            (
                DaEncoder2(DaEncoder::new(params)),
                rand_data(SIZE * MB / DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE),
            )
        })
        .input_counter(|(_, buff)| BytesCount::new(buff.len()))
        .bench_refs(|(encoder, buff)| black_box(nomos_core::da::DaEncoder::encode(encoder, buff)));
}
