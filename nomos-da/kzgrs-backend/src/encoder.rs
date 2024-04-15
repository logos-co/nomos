use crate::common::{hash_column_and_commitment, Chunk, ChunksMatrix, Row};
use crate::global::{DOMAIN, GLOBAL_PARAMETERS};
use ark_poly::univariate::DensePolynomial;
use kzgrs::{
    bytes_to_polynomial, commit_polynomial, encode, generate_element_proof, Commitment,
    Evaluations, KzgRsError, Polynomial, Proof,
};
pub struct DaEncoderParams {
    column_count: usize,
}

impl DaEncoderParams {
    const MAX_BLS12_381_ENCODING_CHUNK_SIZE: usize = 31;

    const fn default_with(column_count: usize) -> Self {
        Self { column_count }
    }
}

pub struct EncodedData {
    data: Vec<u8>,
    chunked_data: ChunksMatrix,
    extended_data: ChunksMatrix,
    row_commitments: Vec<Commitment>,
    rows_proofs: Vec<Vec<Proof>>,
    column_commitments: Vec<Commitment>,
    aggregated_column_commitment: Commitment,
    aggregated_column_proofs: Vec<Proof>,
}

struct DaEncoder {
    params: DaEncoderParams,
}

impl DaEncoder {
    pub const fn new(settings: DaEncoderParams) -> Self {
        Self { params: settings }
    }

    fn chunkify(&self, data: &[u8]) -> ChunksMatrix {
        let size = self.params.column_count * DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE;
        data.windows(size)
            .map(|d| {
                d.windows(DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE)
                    .map(Chunk::from)
                    .collect()
            })
            .collect()
    }

    fn compute_kzg_row_commitments(
        matrix: &ChunksMatrix,
    ) -> Result<Vec<((Evaluations, Polynomial), Commitment)>, KzgRsError> {
        matrix
            .rows()
            .map(|r| {
                bytes_to_polynomial::<{ DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE }>(
                    r.as_bytes().as_ref(),
                    *DOMAIN,
                )
                .and_then(|(evals, poly)| {
                    commit_polynomial(&poly, &GLOBAL_PARAMETERS)
                        .map(|commitment| ((evals, poly), commitment))
                })
            })
            .collect()
    }

    fn rs_encode_row(evaluations: &Evaluations, row: &Polynomial) -> Evaluations {
        encode(row, evaluations, 2, &DOMAIN)
    }

    fn rs_encode_rows(rows: &[(Evaluations, Polynomial)]) -> Vec<Evaluations> {
        rows.iter()
            .map(|(eval, poly)| Self::rs_encode_row(eval, poly))
            .collect()
    }

    fn compute_rows_proofs(
        polynomials: &[Polynomial],
        commitments: &[Commitment],
        size: usize,
    ) -> Result<Vec<Vec<Proof>>, KzgRsError> {
        polynomials
            .iter()
            .zip(commitments.iter())
            .map(|(poly, commitment)| {
                (0..size)
                    .map(|i| generate_element_proof(i, poly, &GLOBAL_PARAMETERS, *DOMAIN))
                    .collect()
            })
            .collect()
    }

    fn compute_kzg_column_commitments(
        matrix: &ChunksMatrix,
    ) -> Result<Vec<((Evaluations, Polynomial), Commitment)>, KzgRsError> {
        Self::compute_kzg_row_commitments(&matrix.transposed())
    }

    fn compute_aggregated_column_commitment(
        matrix: &ChunksMatrix,
        commitments: &[Commitment],
    ) -> Result<((Evaluations, Polynomial), Commitment), KzgRsError> {
        let hashes: Vec<u8> =
            matrix
                .columns()
                .zip(commitments)
                .flat_map(|(column, commitment)| {
                    hash_column_and_commitment::<
                        { DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE },
                    >(&column, commitment)
                })
                .collect();
        let (evals, poly) = bytes_to_polynomial::<
            { DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE },
        >(hashes.as_ref(), *DOMAIN)?;
        let commitment = commit_polynomial(&poly, &GLOBAL_PARAMETERS)?;
        Ok(((evals, poly), commitment))
    }

    fn compute_aggregated_column_proofs(
        polynomial: &Polynomial,
        column_commitments: &[Commitment],
    ) -> Result<Vec<Proof>, KzgRsError> {
        column_commitments
            .iter()
            .enumerate()
            .map(|(i, commitment)| {
                generate_element_proof(i, polynomial, &GLOBAL_PARAMETERS, *DOMAIN)
            })
            .collect()
    }

    fn evals_to_chunk_matrix(evals: &[Evaluations]) -> ChunksMatrix {
        ChunksMatrix(
            evals
                .iter()
                .map(|eval| {
                    Row(eval
                        .evals
                        .iter()
                        .map(|point| {
                            Chunk(point.0 .0.iter().flat_map(|n| n.to_le_bytes()).collect())
                        })
                        .collect())
                })
                .collect(),
        )
    }

    pub fn encode(&self, data: &[u8]) -> Result<EncodedData, kzgrs::KzgRsError> {
        let chunked_data = self.chunkify(data);
        let (row_polynomials, row_commitments): (Vec<_>, Vec<_>) =
            Self::compute_kzg_row_commitments(&chunked_data)?
                .into_iter()
                .unzip();
        let extended_data =
            Self::evals_to_chunk_matrix(Self::rs_encode_rows(&row_polynomials).as_ref());
        let row_polynomials: Vec<_> = row_polynomials.into_iter().map(|(_, p)| p).collect();
        let rows_proofs = Self::compute_rows_proofs(
            &row_polynomials,
            &row_commitments,
            self.params.column_count,
        )?;
        let (column_polynomials, column_commitments): (Vec<_>, Vec<_>) =
            Self::compute_kzg_column_commitments(&extended_data)?
                .into_iter()
                .unzip();
        let ((aggregated_evals, aggregated_polynomial), aggregated_column_commitment) =
            Self::compute_aggregated_column_commitment(&extended_data, &column_commitments)?;
        let aggregated_column_proofs =
            Self::compute_aggregated_column_proofs(&aggregated_polynomial, &column_commitments)?;
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
