use const_hex::FromHex as _;
use num_bigint::BigUint;

use crate::crypto::proofs::quota::DOMAIN_SEPARATION_TAG_FR;

#[test]
fn secret_selection_randomness_dst_encoding() {
    // Blend spec: <https://www.notion.so/nomos-tech/Proof-of-Quota-Specification-215261aa09df81d88118ee22205cbafe?source=copy_link#25e261aa09df802d87edfc54d1d60b80>
    assert_eq!(
        *DOMAIN_SEPARATION_TAG_FR,
        BigUint::from_bytes_be(
            &<[u8; 23]>::from_hex("0x31565f5353454e4d4f444e41525f4e4f495443454c4553").unwrap()
        )
        .into()
    );
}
