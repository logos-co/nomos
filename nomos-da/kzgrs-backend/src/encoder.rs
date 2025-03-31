use std::ops::Div;

use ark_ff::{BigInteger, PrimeField};
use ark_poly::EvaluationDomain;
use kzgrs::{
    bytes_to_polynomial, commit_polynomial,
    common::bytes_to_polynomial_unchecked,
    encode,
    fk20::{fk20_batch_generate_elements_proofs, Toeplitz1Cache},
    Commitment, Evaluations, GlobalParameters, KzgRsError, Polynomial, PolynomialEvaluationDomain,
    Proof, BYTES_PER_FIELD_ELEMENT,
};
#[cfg(feature = "parallel")]
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use crate::{
    common::{hash_commitment, share::DaShare, Chunk, ChunksMatrix, Row},
    global::GLOBAL_PARAMETERS,
};

#[derive(Clone)]
pub struct DaEncoderParams {
    column_count: usize,
    toeplitz1cache: Option<Toeplitz1Cache>,
    global_parameters: GlobalParameters,
}

impl DaEncoderParams {
    pub const MAX_BLS12_381_ENCODING_CHUNK_SIZE: usize = 31;

    #[must_use]
    pub fn new(column_count: usize, with_cache: bool, global_parameters: GlobalParameters) -> Self {
        let toeplitz1cache =
            with_cache.then(|| Toeplitz1Cache::with_size(&global_parameters, column_count));
        Self {
            column_count,
            toeplitz1cache,
            global_parameters,
        }
    }

    pub fn default_with(column_count: usize) -> Self {
        Self {
            column_count,
            toeplitz1cache: None,
            global_parameters: GLOBAL_PARAMETERS.clone(),
        }
    }
}

pub struct EncodedData {
    pub data: Vec<u8>,
    pub chunked_data: ChunksMatrix,
    pub extended_data: ChunksMatrix,
    pub row_commitments: Vec<Commitment>,
    pub rows_proofs: Vec<Vec<Proof>>,
    pub column_commitments: Vec<Commitment>,
    pub aggregated_column_commitment: Commitment,
    pub aggregated_column_proofs: Vec<Proof>,
}

impl EncodedData {
    /// Returns a `DaShare` for the given index.
    /// If the index is out of bounds, returns `None`.
    #[must_use]
    pub fn to_da_share(&self, index: usize) -> Option<DaShare> {
        let column = self.extended_data.columns().nth(index)?;
        Some(DaShare {
            column,
            share_idx: index.try_into().unwrap(),
            column_commitment: self.column_commitments[index],
            aggregated_column_commitment: self.aggregated_column_commitment,
            aggregated_column_proof: self.aggregated_column_proofs[index],
            rows_commitments: self.row_commitments.clone(),
            rows_proofs: self
                .rows_proofs
                .iter()
                .map(|proofs| proofs.get(index).copied().unwrap())
                .collect(),
        })
    }

    #[must_use]
    pub const fn iter(&self) -> EncodedDataIterator {
        EncodedDataIterator::new(self)
    }
}

impl<'a> IntoIterator for &'a EncodedData {
    type Item = DaShare;
    type IntoIter = EncodedDataIterator<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct OwnedEncodedDataIterator {
    encoded_data: EncodedData,
    next_index: usize,
}

impl Iterator for OwnedEncodedDataIterator {
    type Item = DaShare;

    fn next(&mut self) -> Option<Self::Item> {
        let next_da_share = self.encoded_data.to_da_share(self.next_index)?;
        self.next_index += 1;
        Some(next_da_share)
    }
}

pub struct EncodedDataIterator<'a> {
    encoded_data: &'a EncodedData,
    next_index: usize,
}

impl<'a> EncodedDataIterator<'a> {
    #[must_use]
    pub const fn new(encoded_data: &'a EncodedData) -> Self {
        Self {
            encoded_data,
            next_index: 0,
        }
    }
}

impl Iterator for EncodedDataIterator<'_> {
    type Item = DaShare;

    fn next(&mut self) -> Option<Self::Item> {
        let next_da_share = self.encoded_data.to_da_share(self.next_index)?;
        self.next_index += 1;
        Some(next_da_share)
    }
}

pub struct DaEncoder {
    params: DaEncoderParams,
}

impl DaEncoder {
    #[must_use]
    pub const fn new(settings: DaEncoderParams) -> Self {
        Self { params: settings }
    }

    fn chunkify(&self, data: &[u8]) -> ChunksMatrix {
        let chunk_size =
            // column count is divided by two, as later on rows are encoded to twice the size
            self.params.column_count.div(2) * DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE;
        data.chunks(chunk_size)
            .map(|d| {
                d.chunks(DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE)
                    .map(|chunk| {
                        let mut buff = [0u8; BYTES_PER_FIELD_ELEMENT];
                        buff[..DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE]
                            .copy_from_slice(chunk);
                        Chunk::from(buff.as_slice())
                    })
                    .collect()
            })
            .collect()
    }

    #[expect(clippy::type_complexity, reason = "TODO: Address this at some point.")]
    fn compute_kzg_row_commitments(
        global_parameters: &GlobalParameters,
        matrix: &ChunksMatrix,
        polynomial_evaluation_domain: PolynomialEvaluationDomain,
    ) -> Result<Vec<((Evaluations, Polynomial), Commitment)>, KzgRsError> {
        {
            #[cfg(not(feature = "parallel"))]
            {
                matrix.rows()
            }
            #[cfg(feature = "parallel")]
            {
                matrix.par_rows()
            }
        }
        .map(|r| {
            // Using the unchecked version here. Because during the process of chunkifiying
            // we already make sure to have the chunks of proper elements.
            // Also, after rs encoding, we are sure all `Fr` elements already fits within
            // modulus.
            let (evals, poly) = bytes_to_polynomial_unchecked::<BYTES_PER_FIELD_ELEMENT>(
                r.as_bytes().as_ref(),
                polynomial_evaluation_domain,
            );
            commit_polynomial(&poly, global_parameters)
                .map(|commitment| ((evals, poly), commitment))
        })
        .collect()
    }

    fn rs_encode_row(
        row: &Polynomial,
        polynomial_evaluation_domain: PolynomialEvaluationDomain,
    ) -> Evaluations {
        encode(row, polynomial_evaluation_domain)
    }

    fn rs_encode_rows(
        rows: &[Polynomial],
        polynomial_evaluation_domain: PolynomialEvaluationDomain,
    ) -> Vec<Evaluations> {
        {
            #[cfg(not(feature = "parallel"))]
            {
                rows.iter()
            }
            #[cfg(feature = "parallel")]
            {
                rows.par_iter()
            }
        }
        .map(|poly| Self::rs_encode_row(poly, polynomial_evaluation_domain))
        .collect()
    }

    fn compute_rows_proofs(
        global_parameters: &GlobalParameters,
        polynomials: &[Polynomial],
        toeplitz1cache: Option<&Toeplitz1Cache>,
    ) -> Vec<Vec<Proof>> {
        {
            #[cfg(not(feature = "parallel"))]
            {
                polynomials.iter()
            }
            #[cfg(feature = "parallel")]
            {
                polynomials.par_iter()
            }
        }
        .map(|poly| fk20_batch_generate_elements_proofs(poly, global_parameters, toeplitz1cache))
        .collect()
    }

    #[expect(clippy::type_complexity, reason = "TODO: Address this at some point.")]
    fn compute_kzg_column_commitments(
        global_parameters: &GlobalParameters,
        matrix: &ChunksMatrix,
        polynomial_evaluation_domain: PolynomialEvaluationDomain,
    ) -> Result<Vec<((Evaluations, Polynomial), Commitment)>, KzgRsError> {
        Self::compute_kzg_row_commitments(
            global_parameters,
            &matrix.transposed(),
            polynomial_evaluation_domain,
        )
    }

    fn compute_aggregated_column_commitment(
        global_parameters: &GlobalParameters,

        commitments: &[Commitment],
        polynomial_evaluation_domain: PolynomialEvaluationDomain,
    ) -> Result<((Evaluations, Polynomial), Commitment), KzgRsError> {
        let hashes: Vec<u8> = commitments
            .iter()
            .flat_map(hash_commitment::<{ DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE }>)
            .collect();
        let (evals, poly) = bytes_to_polynomial::<
            { DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE },
        >(hashes.as_ref(), polynomial_evaluation_domain)?;
        let commitment = commit_polynomial(&poly, global_parameters)?;
        Ok(((evals, poly), commitment))
    }

    fn compute_aggregated_column_proofs(
        global_parameters: &GlobalParameters,
        polynomial: &Polynomial,
        toeplitz1cache: Option<&Toeplitz1Cache>,
    ) -> Vec<Proof> {
        fk20_batch_generate_elements_proofs(polynomial, global_parameters, toeplitz1cache)
    }

    fn evals_to_chunk_matrix(evals: &[Evaluations]) -> ChunksMatrix {
        ChunksMatrix(
            evals
                .iter()
                .map(|eval| {
                    Row(eval
                        .evals
                        .iter()
                        .map(|point| Chunk(point.into_bigint().to_bytes_le()))
                        .collect())
                })
                .collect(),
        )
    }
}

impl nomos_core::da::DaEncoder for DaEncoder {
    type EncodedData = EncodedData;
    type Error = KzgRsError;

    fn encode(&self, data: &[u8]) -> Result<EncodedData, KzgRsError> {
        let global_parameters = &self.params.global_parameters;
        let chunked_data = self.chunkify(data);
        let row_domain = PolynomialEvaluationDomain::new(self.params.column_count)
            .expect("Domain should be able to build");
        let column_domain = PolynomialEvaluationDomain::new(chunked_data.len())
            .expect("Domain should be able to build");
        let (row_polynomials, row_commitments): (Vec<_>, Vec<_>) =
            Self::compute_kzg_row_commitments(global_parameters, &chunked_data, row_domain)?
                .into_iter()
                .unzip();
        let (_, row_polynomials): (Vec<_>, Vec<_>) = row_polynomials.into_iter().unzip();
        let encoded_evaluations = Self::rs_encode_rows(&row_polynomials, row_domain);
        let extended_data = Self::evals_to_chunk_matrix(&encoded_evaluations);
        let rows_proofs = Self::compute_rows_proofs(
            global_parameters,
            &row_polynomials,
            self.params.toeplitz1cache.as_ref(),
        );
        let (_column_polynomials, column_commitments): (Vec<_>, Vec<_>) =
            Self::compute_kzg_column_commitments(global_parameters, &extended_data, column_domain)?
                .into_iter()
                .unzip();
        let ((_aggregated_evals, aggregated_polynomial), aggregated_column_commitment) =
            Self::compute_aggregated_column_commitment(
                global_parameters,
                &column_commitments,
                row_domain,
            )?;
        let aggregated_column_proofs = Self::compute_aggregated_column_proofs(
            global_parameters,
            &aggregated_polynomial,
            self.params.toeplitz1cache.as_ref(),
        );
        Ok(EncodedData {
            data: data.to_vec(),
            chunked_data,
            extended_data,
            row_commitments,
            rows_proofs,
            column_commitments,
            aggregated_column_commitment,
            aggregated_column_proofs,
        })
    }
}

#[cfg(test)]
pub mod test {
    use std::ops::Div;

    use ark_ff::PrimeField;
    use ark_poly::{EvaluationDomain, GeneralEvaluationDomain};
    use itertools::izip;
    use kzgrs::{
        common::bytes_to_polynomial_unchecked, decode, verify_element_proof, FieldElement,
        PolynomialEvaluationDomain, BYTES_PER_FIELD_ELEMENT,
    };
    use nomos_core::da::DaEncoder as _;
    use once_cell::sync::Lazy;
    use rand::RngCore;

    use crate::{
        encoder::{DaEncoder, DaEncoderParams},
        global::GLOBAL_PARAMETERS,
    };

    pub static DOMAIN_SIZE: usize = 16;
    pub static PARAMS: Lazy<DaEncoderParams> =
        Lazy::new(|| DaEncoderParams::default_with(DOMAIN_SIZE));
    pub static ENCODER: Lazy<DaEncoder> = Lazy::new(|| DaEncoder::new(PARAMS.clone()));

    #[must_use]
    pub fn rand_data(elements_count: usize) -> Vec<u8> {
        let mut buff = vec![0; elements_count * DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE];
        rand::thread_rng().fill_bytes(&mut buff);
        buff
    }

    #[test]
    fn test_chunkify() {
        let params = DaEncoderParams::default_with(2);
        let elements = 10usize;
        let data = rand_data(elements);
        let encoder = DaEncoder::new(params.clone());
        let matrix = encoder.chunkify(&data);
        assert_eq!(matrix.len(), elements.div(params.column_count.div(2)));
        for row in matrix.rows() {
            assert_eq!(row.len(), params.column_count.div(2));
            assert_eq!(row.0[0].len(), BYTES_PER_FIELD_ELEMENT);
        }
    }

    #[test]
    fn test_compute_row_kzg_commitments() {
        let data = rand_data(32);
        let domain = GeneralEvaluationDomain::new(DOMAIN_SIZE).unwrap();
        let matrix = ENCODER.chunkify(data.as_ref());
        let commitments_data =
            DaEncoder::compute_kzg_row_commitments(&GLOBAL_PARAMETERS, &matrix, domain).unwrap();
        assert_eq!(commitments_data.len(), matrix.len());
    }

    #[test]
    fn test_evals_to_chunk_matrix() {
        let data = rand_data(32);
        let matrix = ENCODER.chunkify(data.as_ref());
        let domain = PolynomialEvaluationDomain::new(DOMAIN_SIZE).unwrap();
        let (poly_data, _): (Vec<_>, Vec<_>) =
            DaEncoder::compute_kzg_row_commitments(&GLOBAL_PARAMETERS, &matrix, domain)
                .unwrap()
                .into_iter()
                .unzip();
        let (_, poly_data): (Vec<_>, Vec<_>) = poly_data.into_iter().unzip();
        let extended_rows = DaEncoder::rs_encode_rows(&poly_data, domain);
        let extended_matrix = DaEncoder::evals_to_chunk_matrix(&extended_rows);
        for (r1, r2) in izip!(matrix.iter(), extended_matrix.iter()) {
            for (c1, c2) in izip!(r1.iter(), r2.iter()) {
                assert_eq!(c1, c2);
            }
        }
    }

    #[test]
    fn test_rs_encode_rows() {
        let data = rand_data(32);
        let domain = GeneralEvaluationDomain::new(DOMAIN_SIZE).unwrap();
        let matrix = ENCODER.chunkify(data.as_ref());
        let (poly_data, _): (Vec<_>, Vec<_>) =
            DaEncoder::compute_kzg_row_commitments(&GLOBAL_PARAMETERS, &matrix, domain)
                .unwrap()
                .into_iter()
                .unzip();
        let (evals, polynomials): (Vec<_>, Vec<_>) = poly_data.into_iter().unzip();
        let extended_rows = DaEncoder::rs_encode_rows(&polynomials, domain);
        // check encoding went well, original evaluation points vs extended ones
        for (e1, e2) in izip!(evals.iter(), extended_rows.iter()) {
            for (c1, c2) in izip!(&e1.evals, &e2.evals) {
                assert_eq!(c1, c2);
            }
        }
        let extended_matrix = DaEncoder::evals_to_chunk_matrix(&extended_rows);
        for (r1, r2, evals) in izip!(matrix.iter(), extended_matrix.iter(), extended_rows) {
            assert_eq!(r1.len(), r2.len().div(2));
            for (c1, c2) in izip!(r1.iter(), r2.iter()) {
                assert_eq!(c1, c2);
            }
            let points: Vec<_> = evals.evals.iter().copied().map(Some).collect();
            let poly_2 = decode(r1.len(), &points, domain);
            let (poly_1, _) = bytes_to_polynomial_unchecked::<BYTES_PER_FIELD_ELEMENT>(
                r1.as_bytes().as_ref(),
                domain,
            );
            assert_eq!(poly_1, poly_2);
        }
    }

    #[test]
    fn test_compute_row_proofs() {
        let data = rand_data(32);
        let domain = GeneralEvaluationDomain::new(DOMAIN_SIZE).unwrap();
        let matrix = ENCODER.chunkify(data.as_ref());
        let (poly_data, commitments): (Vec<_>, Vec<_>) =
            DaEncoder::compute_kzg_row_commitments(&GLOBAL_PARAMETERS, &matrix, domain)
                .unwrap()
                .into_iter()
                .unzip();
        let (_evals, polynomials): (Vec<_>, Vec<_>) = poly_data.into_iter().unzip();
        let extended_evaluations = DaEncoder::rs_encode_rows(&polynomials, domain);
        let extended_matrix = DaEncoder::evals_to_chunk_matrix(&extended_evaluations);
        let proofs = DaEncoder::compute_rows_proofs(&GLOBAL_PARAMETERS, &polynomials, None);

        let checks = izip!(matrix.iter(), &commitments, &proofs);
        for (row, commitment, proofs) in checks {
            assert_eq!(proofs.len(), row.len() * 2);
            for (i, chunk) in row.iter().enumerate() {
                let element = FieldElement::from_le_bytes_mod_order(chunk.as_bytes().as_ref());
                assert!(verify_element_proof(
                    i,
                    &element,
                    commitment,
                    &proofs[i],
                    domain,
                    &GLOBAL_PARAMETERS
                ));
            }
        }
        let checks = izip!(extended_matrix.iter(), &commitments, &proofs);
        for (row, commitment, proofs) in checks {
            assert_eq!(proofs.len(), row.len());
            for (i, chunk) in row.iter().enumerate() {
                let element = FieldElement::from_le_bytes_mod_order(chunk.as_bytes().as_ref());
                assert!(verify_element_proof(
                    i,
                    &element,
                    commitment,
                    &proofs[i],
                    domain,
                    &GLOBAL_PARAMETERS
                ));
            }
        }
    }

    #[test]
    fn test_compute_column_kzg_commitments() {
        let data = rand_data(32);
        let domain = GeneralEvaluationDomain::new(DOMAIN_SIZE).unwrap();
        let matrix = ENCODER.chunkify(data.as_ref());
        let commitments_data =
            DaEncoder::compute_kzg_column_commitments(&GLOBAL_PARAMETERS, &matrix, domain).unwrap();
        assert_eq!(commitments_data.len(), matrix.columns().count());
    }

    #[test]
    fn test_compute_aggregated_column_kzg_commitment() {
        let data = rand_data(32);
        let matrix = ENCODER.chunkify(data.as_ref());
        let domain = GeneralEvaluationDomain::new(DOMAIN_SIZE).unwrap();
        let (_, commitments): (Vec<_>, Vec<_>) =
            DaEncoder::compute_kzg_column_commitments(&GLOBAL_PARAMETERS, &matrix, domain)
                .unwrap()
                .into_iter()
                .unzip();
        let _ = DaEncoder::compute_aggregated_column_commitment(
            &GLOBAL_PARAMETERS,
            &commitments,
            domain,
        )
        .unwrap();
    }

    #[test]
    fn test_compute_aggregated_column_kzg_proofs() {
        let data = rand_data(32);
        let matrix = ENCODER.chunkify(data.as_ref());
        let domain = GeneralEvaluationDomain::new(DOMAIN_SIZE).unwrap();
        let (_poly_data, commitments): (Vec<_>, Vec<_>) =
            DaEncoder::compute_kzg_column_commitments(&GLOBAL_PARAMETERS, &matrix, domain)
                .unwrap()
                .into_iter()
                .unzip();
        let ((_evals, polynomial), _aggregated_commitment) =
            DaEncoder::compute_aggregated_column_commitment(
                &GLOBAL_PARAMETERS,
                &commitments,
                domain,
            )
            .unwrap();
        DaEncoder::compute_aggregated_column_proofs(&GLOBAL_PARAMETERS, &polynomial, None);
    }

    #[test]
    fn test_full_encode_flow() {
        let data = rand_data(32);
        let domain = GeneralEvaluationDomain::new(DOMAIN_SIZE).unwrap();
        let encoding_data = ENCODER.encode(&data).unwrap();
        assert_eq!(encoding_data.data, data);
        assert_eq!(encoding_data.row_commitments.len(), 4);
        assert_eq!(encoding_data.column_commitments.len(), 16);
        assert_eq!(encoding_data.rows_proofs.len(), 4);
        assert_eq!(encoding_data.rows_proofs[0].len(), 16);
        assert_eq!(encoding_data.aggregated_column_proofs.len(), 16);
        for (row, proofs, commitment) in izip!(
            encoding_data.extended_data.rows(),
            encoding_data.rows_proofs,
            encoding_data.row_commitments
        ) {
            for (chunk_idx, chunk) in row.iter().enumerate() {
                let element = FieldElement::from_le_bytes_mod_order(chunk.as_bytes().as_ref());
                assert!(verify_element_proof(
                    chunk_idx,
                    &element,
                    &commitment,
                    &proofs[chunk_idx],
                    domain,
                    &GLOBAL_PARAMETERS
                ));
            }
        }
    }

    #[test]
    fn test_encoded_data_iterator() {
        let encoder = &ENCODER;
        let data = vec![
            49u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
            0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
        ];
        let encoded_data = encoder.encode(&data).unwrap();

        let shares = encoded_data.iter();
        assert_eq!(shares.count(), 16);

        let shares = encoded_data.iter();
        assert_eq!(shares.count(), 16);
    }
}
