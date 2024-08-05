// std

use ark_poly::EvaluationDomain;
// crates
use blst::min_sig::{PublicKey, SecretKey};
use itertools::{izip, Itertools};
use kzgrs::common::field_element_from_bytes_le;
use kzgrs::{
    bytes_to_polynomial, commit_polynomial, verify_element_proof, Commitment,
    PolynomialEvaluationDomain, Proof, BYTES_PER_FIELD_ELEMENT,
};

use crate::common::blob::DaBlob;
// internal
use crate::common::{hash_column_and_commitment, Chunk, Column};
use crate::encoder::DaEncoderParams;
use crate::global::GLOBAL_PARAMETERS;

pub struct DaVerifier {
    // TODO: substitute this for an abstraction to sign things over
    pub sk: SecretKey,
    pub index: usize,
}

impl DaVerifier {
    pub fn new(sk: SecretKey, nodes_public_keys: &[PublicKey]) -> Self {
        // TODO: `is_sorted` is experimental, and by contract `nodes_public_keys` should be shorted
        // but not sure how we could enforce it here without re-sorting anyway.
        // assert!(nodes_public_keys.is_sorted());
        let self_pk = sk.sk_to_pk();
        let (index, _) = nodes_public_keys
            .iter()
            .find_position(|&pk| pk == &self_pk)
            .expect("Self pk should be registered");
        Self { sk, index }
    }

    fn verify_column(
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
        let Ok(computed_column_commitment) = commit_polynomial(&polynomial, &GLOBAL_PARAMETERS)
        else {
            return false;
        };
        // 2. if computed column commitment != column commitment, fail
        if &computed_column_commitment != column_commitment {
            return false;
        }
        // 3. compute column hash
        let column_hash = hash_column_and_commitment::<
            { DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE },
        >(column, column_commitment);
        // 4. check proof with commitment and proof over the aggregated column commitment
        let element = field_element_from_bytes_le(column_hash.as_slice());
        verify_element_proof(
            index,
            &element,
            aggregated_column_commitment,
            aggregated_column_proof,
            rows_domain,
            &GLOBAL_PARAMETERS,
        )
    }

    fn verify_chunk(
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
            &GLOBAL_PARAMETERS,
        )
    }

    fn verify_chunks(
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
            if !DaVerifier::verify_chunk(chunk, commitment, proof, index, domain) {
                return false;
            }
        }
        true
    }

    pub fn verify(&self, blob: &DaBlob, rows_domain_size: usize) -> bool {
        let rows_domain = PolynomialEvaluationDomain::new(rows_domain_size)
            .expect("Domain should be able to build");
        let is_column_verified = DaVerifier::verify_column(
            &blob.column,
            &blob.column_commitment,
            &blob.aggregated_column_commitment,
            &blob.aggregated_column_proof,
            self.index,
            rows_domain,
        );
        if !is_column_verified {
            return false;
        }

        let are_chunks_verified = DaVerifier::verify_chunks(
            blob.column.as_ref(),
            &blob.rows_commitments,
            &blob.rows_proofs,
            self.index,
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
    use crate::common::blob::DaBlob;
    use crate::common::{hash_column_and_commitment, Chunk, Column};
    use crate::encoder::test::{rand_data, ENCODER};
    use crate::encoder::DaEncoderParams;
    use crate::global::GLOBAL_PARAMETERS;
    use crate::verifier::DaVerifier;
    use ark_poly::{EvaluationDomain, GeneralEvaluationDomain};
    use blst::min_sig::SecretKey;
    use kzgrs::{
        bytes_to_polynomial, commit_polynomial, generate_element_proof, BYTES_PER_FIELD_ELEMENT,
    };
    use nomos_core::da::DaEncoder as _;
    use rand::{thread_rng, RngCore};

    #[test]
    fn test_verify_column() {
        let column: Column = (0..10).map(|i| Chunk(vec![i; 32])).collect();
        let domain = GeneralEvaluationDomain::new(10).unwrap();
        let (_, column_poly) =
            bytes_to_polynomial::<BYTES_PER_FIELD_ELEMENT>(column.as_bytes().as_slice(), domain)
                .unwrap();
        let column_commitment = commit_polynomial(&column_poly, &GLOBAL_PARAMETERS).unwrap();
        let (aggregated_evals, aggregated_poly) = bytes_to_polynomial::<
            { DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE },
        >(
            hash_column_and_commitment::<{ DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE }>(
                &column,
                &column_commitment,
            )
            .as_slice(),
            domain,
        )
        .unwrap();
        let aggregated_commitment =
            commit_polynomial(&aggregated_poly, &GLOBAL_PARAMETERS).unwrap();
        let column_proof = generate_element_proof(
            0,
            &aggregated_poly,
            &aggregated_evals,
            &GLOBAL_PARAMETERS,
            domain,
        )
        .unwrap();
        assert!(DaVerifier::verify_column(
            &column,
            &column_commitment,
            &aggregated_commitment,
            &column_proof,
            0,
            domain
        ));
    }

    #[test]
    fn test_verify() {
        let encoder = &ENCODER;
        let data = rand_data(32);
        let domain_size = 16usize;
        let mut rng = thread_rng();
        let sks: Vec<SecretKey> = (0..16)
            .map(|_| {
                let mut buff = [0u8; 32];
                rng.fill_bytes(&mut buff);
                SecretKey::key_gen(&buff, &[]).unwrap()
            })
            .collect();
        let verifiers: Vec<DaVerifier> = sks
            .into_iter()
            .enumerate()
            .map(|(index, sk)| DaVerifier { sk, index })
            .collect();
        let encoded_data = encoder.encode(&data).unwrap();
        for (i, column) in encoded_data.extended_data.columns().enumerate() {
            println!("{i}");
            let verifier = &verifiers[i];
            let da_blob = DaBlob {
                column,
                column_commitment: encoded_data.column_commitments[i],
                aggregated_column_commitment: encoded_data.aggregated_column_commitment,
                aggregated_column_proof: encoded_data.aggregated_column_proofs[i],
                rows_commitments: encoded_data.row_commitments.clone(),
                rows_proofs: encoded_data
                    .rows_proofs
                    .iter()
                    .map(|proofs| proofs.get(i).cloned().unwrap())
                    .collect(),
            };
            assert!(verifier.verify(&da_blob, domain_size));
        }
    }
}
