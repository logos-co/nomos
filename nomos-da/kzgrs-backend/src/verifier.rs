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
}
