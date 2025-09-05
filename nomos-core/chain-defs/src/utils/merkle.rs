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

/// Calculates a 32-byte Merkle root for the given elements, with deterministic
/// padding.
///
/// Padding rules:
/// - The leaf count expands to the next power of two, respecting at least
///   `pad_to`, and treating an empty input as having one leaf.
/// - Missing leaves are filled using the default value of `T`, ensuring stable
///   roots.
///
/// Parameters:
/// - `elements`: Items included in the tree; each converts to a 32-byte leaf.
/// - `pad_to`: Minimum leaf count before rounding up to a power of two.
///
/// Returns:
/// - The Merkle root as a 32-byte array.
///
/// Notes:
/// - Using a fixed `pad_to` keeps the tree height consistent across varying
///   input sizes.
/// - For an empty `elements`, the root corresponds to a single default leaf.
pub fn calculate_merkle_root<T>(elements: &[T], pad_to: usize) -> [u8; 32]
where
    T: Into<[u8; 32]> + Default + Clone,
{
    let mut leaves: Vec<[u8; 32]> = elements
        .iter()
        .cloned()
        .map(|element| leaf(&element.into()))
        .collect();

    let target_size = leaves.len().max(pad_to).max(1).next_power_of_two();

    if leaves.len() < target_size {
        let zero_leaf = leaf(&T::default().into());
        leaves.resize(target_size, zero_leaf);
    }

    while leaves.len() > 1 {
        leaves = leaves
            .chunks(2)
            .map(|pair| node(pair[0], pair[1]))
            .collect();
    }

    leaves[0]
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
}
