// std
use ark_serialize::CanonicalSerialize;
use std::io::Cursor;
// crates
use blake2::digest::{Update, VariableOutput};
use blst::min_sig::Signature;
use sha3::{Digest, Sha3_256};
// internal
use kzgrs::Commitment;

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct Chunk(pub Vec<u8>);

#[derive(Debug)]
pub struct Row(pub Vec<Chunk>);

#[derive(Debug)]
pub struct Column(pub Vec<Chunk>);

pub struct ChunksMatrix(pub Vec<Row>);

impl Chunk {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn as_bytes(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    pub const fn empty() -> Self {
        Self(vec![])
    }
}

impl From<&[u8]> for Chunk {
    fn from(value: &[u8]) -> Self {
        Self(value.to_vec())
    }
}

impl Row {
    pub fn iter(&self) -> impl Iterator<Item = &Chunk> {
        self.0.iter()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn as_bytes(&self) -> Vec<u8> {
        self.0.iter().flat_map(Chunk::as_bytes).collect()
    }
}

impl Column {
    #[allow(unused)]
    pub fn iter(&self) -> impl Iterator<Item = &Chunk> {
        self.0.iter()
    }
    #[allow(unused)]
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn as_bytes(&self) -> Vec<u8> {
        self.0.iter().flat_map(Chunk::as_bytes).collect()
    }
}

impl FromIterator<Chunk> for Row {
    fn from_iter<T: IntoIterator<Item = Chunk>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl FromIterator<Chunk> for Column {
    fn from_iter<T: IntoIterator<Item = Chunk>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl AsRef<[Chunk]> for Row {
    fn as_ref(&self) -> &[Chunk] {
        &self.0
    }
}

impl AsRef<[Chunk]> for Column {
    fn as_ref(&self) -> &[Chunk] {
        &self.0
    }
}

impl ChunksMatrix {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn rows(&self) -> impl Iterator<Item = &Row> + '_ {
        self.0.iter()
    }
    pub fn columns(&self) -> impl Iterator<Item = Column> + '_ {
        let size = self.0.first().map(|r| r.0.len()).unwrap_or(0);
        (0..size).map(|i| {
            self.0
                .iter()
                .map(|row| row.0.get(i).cloned().unwrap_or_else(Chunk::empty))
                .collect::<Column>()
        })
    }

    pub fn transposed(&self) -> Self {
        Self(self.columns().map(|c| Row(c.0)).collect())
    }

    pub fn iter(&self) -> impl Iterator<Item = &Row> {
        self.0.iter()
    }
}

impl FromIterator<Row> for ChunksMatrix {
    fn from_iter<T: IntoIterator<Item = Row>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

pub fn hash_column_and_commitment<const HASH_SIZE: usize>(
    column: &Column,
    commitment: &Commitment,
) -> [u8; HASH_SIZE] {
    let mut hasher = blake2::Blake2bVar::new(HASH_SIZE)
        .unwrap_or_else(|e| panic!("Blake2b should work for size {HASH_SIZE}, {e}"));
    hasher.update(column.as_bytes().as_ref());
    hasher.update(commitment_to_bytes(commitment).as_ref());
    hasher
        .finalize_boxed()
        .to_vec()
        .try_into()
        .unwrap_or_else(|_| panic!("Size is guaranteed by constant {HASH_SIZE:?}"))
}

pub fn build_attestation_message(
    aggregated_column_commitment: &Commitment,
    rows_commitments: &[Commitment],
) -> Vec<u8> {
    let mut hasher = Sha3_256::new();
    Digest::update(
        &mut hasher,
        commitment_to_bytes(aggregated_column_commitment),
    );
    for c in rows_commitments {
        Digest::update(&mut hasher, commitment_to_bytes(c));
    }
    hasher.finalize().to_vec()
}

pub fn commitment_to_bytes(commitment: &Commitment) -> Vec<u8> {
    let mut buff = Cursor::new(vec![]);
    commitment
        .serialize_uncompressed(&mut buff)
        .expect("Serialization of commitment should work");
    buff.into_inner()
}

#[derive(Debug)]
pub struct Attestation {
    pub signature: Signature,
}
