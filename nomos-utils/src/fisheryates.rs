use rand::{Rng as _, SeedableRng as _};
use rand_chacha::ChaCha20Rng;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct FisherYatesShuffle {
    pub entropy: [u8; 32],
}

impl FisherYatesShuffle {
    #[must_use]
    pub const fn new(entropy: [u8; 32]) -> Self {
        Self { entropy }
    }

    pub fn shuffle<T: Clone>(elements: &mut [T], entropy: [u8; 32]) {
        let mut rng = ChaCha20Rng::from_seed(entropy);
        // Implementation of fisher yates shuffling
        // https://en.wikipedia.org/wiki/Fisher%E2%80%93Yates_shuffle
        for i in (1..elements.len()).rev() {
            let j = rng.random_range(0..=i);
            elements.swap(i, j);
        }
    }
}
