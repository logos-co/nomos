use nomos_proof_statements::leadership::LeaderPublic;

pub trait LeaderProof {
    fn verify(&self, public_inputs: &LeaderPublic) -> bool;
    // Entropy contribution to the epoch nonce
    fn entropy(&self) -> [u8; 32];
    // TODO: missing rewards
}
