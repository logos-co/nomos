use rand::{Rng as _, RngCore};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct FisherYates;

impl FisherYates {
    pub fn shuffle<R: RngCore, T: Clone>(elements: &mut [T], rng: &mut R) {
        // Implementation of fisher yates shuffling
        // https://en.wikipedia.org/wiki/Fisher%E2%80%93Yates_shuffle
        for i in (1..elements.len()).rev() {
            let j = rng.random_range(0..=i);
            elements.swap(i, j);
        }
    }
}
