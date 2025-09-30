use crate::crypto::{Digest as _, Hasher};

#[derive(Clone)]
pub enum MerkleNode<T> {
    Left(T),
    Right(T),
}

impl<T> MerkleNode<T> {
    pub const fn item(&self) -> &T {
        match self {
            Self::Left(v) | Self::Right(v) => v,
        }
    }
}

pub type MerklePath<T> = Vec<MerkleNode<T>>;

#[must_use]
pub fn leaf(data: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(b"NOMOS_MERKLE_LEAF");
    hasher.update(data);
    hasher.finalize().into()
}

pub fn node(left: impl AsRef<[u8]>, right: impl AsRef<[u8]>) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(b"NOMOS_MERKLE_NODE");
    hasher.update(left.as_ref());
    hasher.update(right.as_ref());
    hasher.finalize().into()
}

pub fn calculate_merkle_root<T>(elements: &[T], pad_to: usize) -> [u8; 32]
where
    T: Into<[u8; 32]> + Default + Clone,
{
    let mut leaves: Vec<[u8; 32]> = elements
        .iter()
        .cloned()
        .map(|element| leaf(&element.into()))
        .collect();

    if leaves.len() < pad_to {
        let zero_element = T::default();
        let zero_leaf = leaf(&zero_element.into());
        leaves.resize(pad_to, zero_leaf);
    }

    let leaves_count = leaves.len();

    assert!(
        leaves_count >= 2 && leaves_count.is_power_of_two(),
        "Input must be full binary tree"
    );

    (0..leaves_count.ilog2()).fold(leaves, |level, _| {
        level.chunks(2).map(|pair| node(pair[0], pair[1])).collect()
    })[0]
}

#[cfg(test)]
mod tests {
    use groth16::Fr;

    use super::*;
    use crate::mantle::TxHash;

    #[test]
    fn test_root_two_elements() {
        let elements = vec![TxHash::from(Fr::from(1u64)), TxHash::from(Fr::from(2u64))];
        let result = calculate_merkle_root(&elements, 2);

        let bytes1: [u8; 32] = elements[0].into();
        let bytes2: [u8; 32] = elements[1].into();
        let leaf1 = leaf(&bytes1);
        let leaf2 = leaf(&bytes2);
        let expected = node(leaf1, leaf2);

        assert_eq!(result, expected);
    }

    #[test]
    fn test_root_with_padding() {
        let elements = vec![TxHash::from(Fr::from(1u64)), TxHash::from(Fr::from(2u64))];
        let result = calculate_merkle_root(&elements, 4);

        let bytes1: [u8; 32] = elements[0].into();
        let bytes2: [u8; 32] = elements[1].into();
        let zero_hash = TxHash::default();
        let zero_bytes: [u8; 32] = zero_hash.into();

        let leaf1 = leaf(&bytes1);
        let leaf2 = leaf(&bytes2);
        let leaf3 = leaf(&zero_bytes); // padding
        let leaf4 = leaf(&zero_bytes); // padding

        let branch1 = node(leaf1, leaf2);
        let branch2 = node(leaf3, leaf4);
        let expected = node(branch1, branch2);

        assert_eq!(result, expected);
    }

    #[test]
    #[should_panic(expected = "Input must be full binary tree")]
    fn test_root_invalid_padding_size() {
        let elements = vec![TxHash::from(Fr::from(1u64)), TxHash::from(Fr::from(2u64))];
        calculate_merkle_root(&elements, 3);
    }
}
