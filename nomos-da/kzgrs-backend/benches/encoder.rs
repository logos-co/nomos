use divan::counter::BytesCount;
use divan::Bencher;
use kzgrs_backend::encoder::{DaEncoder, DaEncoderParams};
use rand::RngCore;
use std::hint::black_box;

fn main() {
    divan::main()
}

const MB: usize = 1024;

pub fn rand_data(elements_count: usize) -> Vec<u8> {
    let mut buff = vec![0; elements_count * DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE];
    rand::thread_rng().fill_bytes(&mut buff);
    buff
}
#[divan::bench(consts = [32, 64, 128, 256, 512, 1024], args = [128, 256, 512, 1024, 2048, 4096], sample_count = 1, sample_size = 1)]
fn encode<const SIZE: usize>(bencher: Bencher, column_size: usize) {
    bencher
        .with_inputs(|| {
            let params = DaEncoderParams::new(column_size, true);
            (
                DaEncoder::new(params),
                rand_data(SIZE * MB / DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE),
            )
        })
        .input_counter(|(_, buff)| BytesCount::new(buff.len()))
        .bench_refs(|(encoder, buff)| black_box(encoder.encode(buff)));
}
