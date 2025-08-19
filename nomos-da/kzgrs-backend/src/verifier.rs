use ark_ff::PrimeField as _;
use ark_poly::EvaluationDomain as _;
use kzgrs::{FieldElement, GlobalParameters, PolynomialEvaluationDomain};

use crate::common::{
    share::{DaLightShare, DaSharesCommitments},
    Chunk,
};

pub struct DaVerifier {
    pub global_parameters: GlobalParameters,
}

impl DaVerifier {
    #[must_use]
    pub const fn new(global_parameters: GlobalParameters) -> Self {
        Self { global_parameters }
    }

    #[must_use]
    pub fn verify(
        &self,
        share: &DaLightShare,
        commitments: &DaSharesCommitments,
        rows_domain_size: usize,
    ) -> bool {
        let column: Vec<FieldElement> = share
            .column
            .iter()
            .map(|Chunk(b)| FieldElement::from_le_bytes_mod_order(b))
            .collect();
        let rows_domain = PolynomialEvaluationDomain::new(rows_domain_size)
            .expect("Domain should be able to build");
        kzgrs::bdfg_proving::verify_column(
            share.share_idx as usize,
            &column,
            &commitments.rows_commitments,
            &share.combined_column_proof,
            rows_domain,
            &self.global_parameters,
        )
    }
}

#[cfg(test)]
mod test {
    use std::hint::black_box;

    use nomos_core::da::{blob::Share as _, DaEncoder as _};

    use crate::{
        encoder::{test::rand_data, DaEncoder, DaEncoderParams},
        global::GLOBAL_PARAMETERS,
        verifier::DaVerifier,
    };

    #[test]
    fn test_verify() {
        let encoder = DaEncoder::new(DaEncoderParams::default_with(2));
        let data = rand_data(32);
        let domain_size = 2usize;
        let verifier = DaVerifier::new(GLOBAL_PARAMETERS.clone());
        let encoded_data = encoder.encode(&data).unwrap();
        for share in &encoded_data {
            let (light_share, commitments) = share.into_share_and_commitments();
            assert!(verifier.verify(&light_share, &commitments, domain_size));
        }
    }

    // TODO: Remove this when we have the proper benches in the proofs
    #[cfg(target_arch = "x86_64")]
    #[expect(
        clippy::undocumented_unsafe_blocks,
        reason = "This test is just to measure cpu and should be run manually"
    )]
    #[ignore = "This test is just for calculation the cycles for the above set of proofs. This will be moved to the pertinent proof in the future."]
    #[test]
    fn test_verify_cycles() {
        let iters = 500u64;

        let encoder = DaEncoder::new(DaEncoderParams::default_with(2));
        let data = rand_data(32);
        let domain_size = 2usize;
        let verifier = DaVerifier::new(GLOBAL_PARAMETERS.clone());
        let encoded_data = encoder.encode(&data).unwrap();

        let t0 = unsafe { core::arch::x86_64::_rdtsc() };
        for _ in 0..iters {
            for share in &encoded_data {
                let (light_share, commitments) = share.into_share_and_commitments();
                black_box(verifier.verify(&light_share, &commitments, domain_size));
            }
        }
        let t1 = unsafe { core::arch::x86_64::_rdtsc() };

        let cycles_diff = t1 - t0;
        let cycles_per_run = (t1 - t0) / iters;
        println!("=== kzgrs-da-verify results ===");
        println!("  - iterations     : {iters:>20} runs");
        println!("  - cycles total   : {cycles_diff:>20} cpu cycles");
        println!("  - cycles/run     : {cycles_per_run:>20} cpu cycles");
        println!("================================");
    }
}
