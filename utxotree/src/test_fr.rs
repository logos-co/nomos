use num_bigint::BigUint;
use poseidon2::Fr;
use rand::RngCore;

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct TestFr(Fr);

impl TestFr {
    pub fn from_rng<Rng: RngCore>(rng: &mut Rng) -> Self {
        Self(BigUint::from(rng.next_u64()).into())
    }
}
impl AsRef<Fr> for TestFr {
    fn as_ref(&self) -> &Fr {
        &self.0
    }
}
