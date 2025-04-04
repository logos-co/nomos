use ark_poly::EvaluationDomain;
use itertools::{izip, Itertools};
use kzgrs::{
    bytes_to_polynomial, commit_polynomial, common::field_element_from_bytes_le,
    verify_element_proof, Commitment, GlobalParameters, PolynomialEvaluationDomain, Proof,
    BYTES_PER_FIELD_ELEMENT,
};

use crate::{
    common::{
        hash_commitment,
        share::{DaLightShare, DaSharesCommitments},
        Chunk, Column,
    },
    encoder::DaEncoderParams,
};

pub struct DaVerifier {
    global_parameters: GlobalParameters,
}

impl DaVerifier {
    #[must_use]
    pub const fn new(global_parameters: GlobalParameters) -> Self {
        Self { global_parameters }
    }

    fn verify_column(
        global_parameters: &GlobalParameters,
        column: &Column,
        column_commitment: &Commitment,
        aggregated_column_commitment: &Commitment,
        aggregated_column_proof: &Proof,
        index: usize,
        rows_domain: PolynomialEvaluationDomain,
    ) -> bool {
        let column_domain =
            PolynomialEvaluationDomain::new(column.len()).expect("Domain should be able to build");
        // 1. compute commitment for column
        let Ok((_, polynomial)) = bytes_to_polynomial::<BYTES_PER_FIELD_ELEMENT>(
            column.as_bytes().as_slice(),
            column_domain,
        ) else {
            return false;
        };
        let Ok(computed_column_commitment) = commit_polynomial(&polynomial, global_parameters)
        else {
            return false;
        };
        // 2. if computed column commitment != column commitment, fail
        if &computed_column_commitment != column_commitment {
            return false;
        }
        // 3. compute column hash
        let commitment_hash = hash_commitment::<
            { DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE },
        >(column_commitment);
        // 4. check proof with commitment and proof over the aggregated column
        //    commitment
        let element = field_element_from_bytes_le(commitment_hash.as_slice());
        verify_element_proof(
            index,
            &element,
            aggregated_column_commitment,
            aggregated_column_proof,
            rows_domain,
            global_parameters,
        )
    }

    fn verify_chunk(
        global_parameters: &GlobalParameters,
        chunk: &Chunk,
        commitment: &Commitment,
        proof: &Proof,
        index: usize,
        domain: PolynomialEvaluationDomain,
    ) -> bool {
        let element = field_element_from_bytes_le(chunk.as_bytes().as_slice());
        verify_element_proof(
            index,
            &element,
            commitment,
            proof,
            domain,
            global_parameters,
        )
    }

    fn verify_chunks(
        global_parameters: &GlobalParameters,
        chunks: &[Chunk],
        commitments: &[Commitment],
        proofs: &[Proof],
        index: usize,
        domain: PolynomialEvaluationDomain,
    ) -> bool {
        if ![chunks.len(), commitments.len(), proofs.len()]
            .iter()
            .all_equal()
        {
            return false;
        }
        for (chunk, commitment, proof) in izip!(chunks, commitments, proofs) {
            if !Self::verify_chunk(global_parameters, chunk, commitment, proof, index, domain) {
                return false;
            }
        }
        true
    }

    #[must_use]
    pub fn verify(
        &self,
        commitments: &DaSharesCommitments,
        light_share: &DaLightShare,
        rows_domain_size: usize,
    ) -> bool {
        let rows_domain = PolynomialEvaluationDomain::new(rows_domain_size)
            .expect("Domain should be able to build");
        let share_col_idx = &light_share.share_idx;
        let index = share_col_idx;

        let is_column_verified = Self::verify_column(
            &self.global_parameters,
            &light_share.column,
            &light_share.column_commitment,
            &commitments.aggregated_column_commitment,
            &light_share.aggregated_column_proof,
            *index as usize,
            rows_domain,
        );
        if !is_column_verified {
            return false;
        }

        let are_chunks_verified = Self::verify_chunks(
            &self.global_parameters,
            light_share.column.as_ref(),
            &commitments.rows_commitments,
            &light_share.rows_proofs,
            *index as usize,
            rows_domain,
        );
        if !are_chunks_verified {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod test {
    use std::sync::LazyLock;

    use ark_bls12_381::Fr;
    use ark_poly::{EvaluationDomain, GeneralEvaluationDomain};
    use kzgrs::{
        bytes_to_polynomial, commit_polynomial, generate_element_proof,
        global_parameters_from_randomness, Commitment, GlobalParameters,
        PolynomialEvaluationDomain, Proof, BYTES_PER_FIELD_ELEMENT,
    };
    use nomos_core::da::{blob::Share, DaEncoder};

    use crate::{
        common::{hash_commitment, share::DaShare, Chunk, Column},
        encoder::{
            test::{rand_data, ENCODER},
            DaEncoderParams,
        },
        global::GLOBAL_PARAMETERS,
        verifier::DaVerifier,
    };

    pub struct ColumnVerifyData {
        pub column: Column,
        pub column_commitment: Commitment,
        pub aggregated_commitment: Commitment,
        pub column_proof: Proof,
        pub domain: GeneralEvaluationDomain<Fr>,
    }

    fn prepare_column(
        with_new_global_params: bool,
    ) -> Result<ColumnVerifyData, Box<dyn std::error::Error>> {
        pub static NEW_GLOBAL_PARAMETERS: LazyLock<GlobalParameters> = LazyLock::new(|| {
            let mut rng = rand::thread_rng();
            global_parameters_from_randomness(&mut rng)
        });

        let global_params = if with_new_global_params {
            &NEW_GLOBAL_PARAMETERS
        } else {
            &GLOBAL_PARAMETERS
        };

        let column: Column = (0..10).map(|i| Chunk(vec![i; 32])).collect();
        let domain = GeneralEvaluationDomain::new(10).unwrap();
        let (_, column_poly) =
            bytes_to_polynomial::<BYTES_PER_FIELD_ELEMENT>(column.as_bytes().as_slice(), domain)?;
        let column_commitment = commit_polynomial(&column_poly, global_params)?;

        let (aggregated_evals, aggregated_poly) =
            bytes_to_polynomial::<{ DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE }>(
                hash_commitment::<{ DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE }>(
                    &column_commitment,
                )
                .as_slice(),
                domain,
            )?;

        let aggregated_commitment = commit_polynomial(&aggregated_poly, global_params)?;

        let column_proof = generate_element_proof(
            0,
            &aggregated_poly,
            &aggregated_evals,
            global_params,
            domain,
        )?;

        Ok(ColumnVerifyData {
            column,
            column_commitment,
            aggregated_commitment,
            column_proof,
            domain,
        })
    }

    #[test]
    fn test_verify_column() {
        let column_data = prepare_column(false).unwrap();

        assert!(DaVerifier::verify_column(
            &GLOBAL_PARAMETERS,
            &column_data.column,
            &column_data.column_commitment,
            &column_data.aggregated_commitment,
            &column_data.column_proof,
            0,
            column_data.domain
        ));
    }

    #[test]
    fn test_verify_column_error_cases() {
        // Test bytes_to_polynomial() returned error
        let column_data = prepare_column(false).unwrap();

        let column2: Column = (0..10)
            .flat_map(|i| {
                if i % 2 == 0 {
                    vec![Chunk(vec![i; 16])]
                } else {
                    vec![Chunk(vec![i; 32])]
                }
            })
            .collect();

        assert!(bytes_to_polynomial::<BYTES_PER_FIELD_ELEMENT>(
            column2.as_bytes().as_slice(),
            column_data.domain
        )
        .is_err());

        assert!(!DaVerifier::verify_column(
            &GLOBAL_PARAMETERS,
            &column2,
            &column_data.column_commitment,
            &column_data.aggregated_commitment,
            &column_data.column_proof,
            0,
            column_data.domain
        ));

        // Test alter GLOBAL_PARAMETERS so that computed_column_commitment !=
        // column_commitment
        let column_data2 = prepare_column(true).unwrap();

        assert!(!DaVerifier::verify_column(
            &GLOBAL_PARAMETERS,
            &column_data2.column,
            &column_data2.column_commitment,
            &column_data2.aggregated_commitment,
            &column_data2.column_proof,
            0,
            column_data2.domain
        ));
    }

    #[test]
    fn test_verify_chunks_error_cases() {
        let encoder = &ENCODER;
        let data = rand_data(32);
        let domain_size = 16usize;
        let rows_domain = PolynomialEvaluationDomain::new(domain_size).unwrap();
        let encoded_data = encoder.encode(&data).unwrap();
        let column = encoded_data.extended_data.columns().next().unwrap();
        let index = 0usize;

        let da_share = DaShare {
            column,
            share_idx: index.try_into().unwrap(),
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
        // Happy case
        let chunks_verified = DaVerifier::verify_chunks(
            &GLOBAL_PARAMETERS,
            da_share.column.as_ref(),
            &da_share.rows_commitments,
            &da_share.rows_proofs,
            index,
            rows_domain,
        );
        assert!(chunks_verified);

        // Chunks altered
        let mut column_w_missing_chunk = da_share.column.as_ref().to_vec();
        column_w_missing_chunk.pop();

        let chunks_not_verified = !DaVerifier::verify_chunks(
            &GLOBAL_PARAMETERS,
            column_w_missing_chunk.as_ref(),
            &da_share.rows_commitments,
            &da_share.rows_proofs,
            index,
            rows_domain,
        );
        assert!(chunks_not_verified);

        // Proofs altered
        let mut modified_proofs = da_share.rows_proofs.clone();
        modified_proofs.swap(0, 1);

        let chunks_not_verified = !DaVerifier::verify_chunks(
            &GLOBAL_PARAMETERS,
            da_share.column.as_ref(),
            &da_share.rows_commitments,
            &modified_proofs,
            index,
            rows_domain,
        );
        assert!(chunks_not_verified);

        // Commitments altered
        let mut modified_commitments = da_share.rows_commitments.clone();
        modified_commitments.swap(0, 1);

        let chunks_not_verified = !DaVerifier::verify_chunks(
            &GLOBAL_PARAMETERS,
            da_share.column.as_ref(),
            &modified_commitments,
            &da_share.rows_proofs,
            index,
            rows_domain,
        );
        assert!(chunks_not_verified);
    }

    #[test]
    fn test_verify() {
        let encoder = &ENCODER;
        let data = rand_data(32);
        let domain_size = 16usize;
        let verifiers: Vec<DaVerifier> = (0..16)
            .map(|_| DaVerifier::new(GLOBAL_PARAMETERS.clone()))
            .collect();
        let encoded_data = encoder.encode(&data).unwrap();
        for (i, column) in encoded_data.extended_data.columns().enumerate() {
            println!("{i}");
            let verifier = &verifiers[i];
            let da_share = DaShare {
                column,
                share_idx: i
                    .try_into()
                    .expect("Column index shouldn't overflow the target type"),
                column_commitment: encoded_data.column_commitments[i],
                aggregated_column_commitment: encoded_data.aggregated_column_commitment,
                aggregated_column_proof: encoded_data.aggregated_column_proofs[i],
                rows_commitments: encoded_data.row_commitments.clone(),
                rows_proofs: encoded_data
                    .rows_proofs
                    .iter()
                    .map(|proofs| proofs.get(i).copied().unwrap())
                    .collect(),
            };
            let (light_share, commitments) = da_share.into_share_and_commitments();
            assert!(verifier.verify(&commitments, &light_share, domain_size));
        }
    }
}
