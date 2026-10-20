// TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
// LEADER_CLAIM is retired.
// use lb_poc::{PoCProof, PoCVerifierInput};
use lb_groth16::{CompressedGroth16Proof, Fr};
use lb_program_spend::{ZkProgramSpendProof, ZkProgramSpendVerifierInput};
// TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
// use lb_zksign::{ZkSignProof, ZkSignVerifierInputs};
use lb_zktransfer::{ZkTransferProof, ZkTransferVerifierInput};

use crate::mantle::programs::{PROGRAM_PUBLIC_INPUTS, RegisteredProgram};

/// A proof that can be handed off to a [`DeferredProofs`] batch.
pub trait DeferrableProof {
    fn defer_into(self, batch: &mut DeferredProofs);
}

/// A deferred proof verification while processing an operation
#[derive(Debug)]
pub struct DeferredProof<Proof, Inputs> {
    proof: Proof,
    inputs: Inputs,
}

impl<Proof, Inputs> DeferredProof<Proof, Inputs> {
    #[must_use]
    pub const fn new(proof: Proof, inputs: Inputs) -> Self {
        Self { proof, inputs }
    }

    pub const fn proof(&self) -> &Proof {
        &self.proof
    }

    pub const fn inputs(&self) -> &Inputs {
        &self.inputs
    }

    #[must_use]
    pub fn into_parts(self) -> (Proof, Inputs) {
        (self.proof, self.inputs)
    }
}

// TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
// pub type DeferredZkSig = DeferredProof<ZkSignProof, ZkSignVerifierInputs>;
// TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
// LEADER_CLAIM is retired.
// pub type DeferredLeaderClaim = DeferredProof<PoCProof, PoCVerifierInput>;
pub type DeferredZkTransfer = DeferredProof<ZkTransferProof, ZkTransferVerifierInput>;
pub type DeferredZkProgramSpend = DeferredProof<ZkProgramSpendProof, ZkProgramSpendVerifierInput>;
pub type DeferredProgramProof = DeferredProof<CompressedGroth16Proof, ProgramProofInput>;

#[derive(Clone, Debug)]
pub struct ProgramProofInput {
    pub program: RegisteredProgram,
    pub public: [Fr; PROGRAM_PUBLIC_INPUTS],
}

// TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
/*
impl DeferrableProof for DeferredZkSig {
    fn defer_into(self, batch: &mut DeferredProofs) {
        batch.zk_sigs.push(self);
    }
}
*/

impl DeferrableProof for DeferredZkTransfer {
    fn defer_into(self, batch: &mut DeferredProofs) {
        batch.zk_transfers.push(self);
    }
}

impl DeferrableProof for DeferredZkProgramSpend {
    fn defer_into(self, batch: &mut DeferredProofs) {
        batch.program_spends.push(self);
    }
}

impl DeferrableProof for DeferredProgramProof {
    fn defer_into(self, batch: &mut DeferredProofs) {
        batch.program_proofs.push(self);
    }
}

// The proofs of the steps of an inscription.
#[derive(Debug, Default)]
pub struct InscriptionDeferredProofs {
    // The ZkTransfers of the user steps and of the payments, in order.
    pub zk_transfers: Vec<DeferredZkTransfer>,
    pub program_spends: Vec<DeferredZkProgramSpend>,
    pub program_proofs: Vec<DeferredProgramProof>,
}

impl DeferrableProof for InscriptionDeferredProofs {
    fn defer_into(self, batch: &mut DeferredProofs) {
        batch.zk_transfers.extend(self.zk_transfers);
        batch.program_spends.extend(self.program_spends);
        batch.program_proofs.extend(self.program_proofs);
    }
}

// TODO(RFC-481): the inscription defers InscriptionDeferredProofs.
/*
impl DeferrableProof for Vec<DeferredZkTransfer> {
    fn defer_into(self, batch: &mut DeferredProofs) {
        batch.zk_transfers.extend(self);
    }
}
*/

// TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
// LEADER_CLAIM is retired.
/*
impl DeferrableProof for DeferredLeaderClaim {
    fn defer_into(self, batch: &mut DeferredProofs) {
        batch.leader_claims.push(self);
    }
}
*/

/// A group of deferred proof verifications while applying a block
#[derive(Default)]
#[must_use]
pub struct DeferredProofs {
    // TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
    // zk_sigs: Vec<DeferredZkSig>,
    // TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
    // LEADER_CLAIM is retired.
    // leader_claims: Vec<DeferredLeaderClaim>,
    zk_transfers: Vec<DeferredZkTransfer>,
    program_spends: Vec<DeferredZkProgramSpend>,
    program_proofs: Vec<DeferredProgramProof>,
}

impl DeferredProofs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn extend(&mut self, other: Self) {
        // TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
        // self.zk_sigs.extend(other.zk_sigs);
        // TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
        // LEADER_CLAIM is retired.
        // self.leader_claims.extend(other.leader_claims);
        self.zk_transfers.extend(other.zk_transfers);
        self.program_spends.extend(other.program_spends);
        self.program_proofs.extend(other.program_proofs);
    }

    pub fn verify(self) -> Result<(), Error> {
        // TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
        // LEADER_CLAIM is retired.
        // Self::verify_zk_sigs(self.zk_sigs)?;
        // Self::verify_leader_claims(self.leader_claims)
        // TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
        // Self::verify_zk_sigs(self.zk_sigs)?;
        Self::verify_zk_transfers(self.zk_transfers)?;
        Self::verify_program_spends(self.program_spends)?;
        Self::verify_program_proofs(self.program_proofs)
    }

    // TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
    /*
    fn verify_zk_sigs(proofs: Vec<DeferredZkSig>) -> Result<(), Error> {
        if proofs.is_empty() {
            return Ok(());
        }

        let proof_parts = proofs
            .into_iter()
            .map(DeferredZkSig::into_parts)
            .collect::<Vec<_>>();
        match lb_zksign::batch_verify(proof_parts.as_slice()) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::InvalidZkSignatures),
            Err(e) => Err(Error::MalformedZkSignature(format!("{e:?}"))),
        }
    }
    */

    fn verify_zk_transfers(proofs: Vec<DeferredZkTransfer>) -> Result<(), Error> {
        if proofs.is_empty() {
            return Ok(());
        }

        let proof_parts = proofs
            .into_iter()
            .map(DeferredZkTransfer::into_parts)
            .collect::<Vec<_>>();
        match lb_zktransfer::batch_verify(proof_parts.as_slice()) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::InvalidZkTransfers),
            Err(e) => Err(Error::MalformedZkTransfer(format!("{e:?}"))),
        }
    }

    fn verify_program_spends(proofs: Vec<DeferredZkProgramSpend>) -> Result<(), Error> {
        if proofs.is_empty() {
            return Ok(());
        }

        let proof_parts = proofs
            .into_iter()
            .map(DeferredZkProgramSpend::into_parts)
            .collect::<Vec<_>>();
        match lb_program_spend::batch_verify(proof_parts.as_slice()) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::InvalidProgramSpends),
            Err(e) => Err(Error::MalformedProgramSpend(format!("{e:?}"))),
        }
    }

    // Each program proof is verified alone, it cannot share the batch of another
    // key.
    fn verify_program_proofs(proofs: Vec<DeferredProgramProof>) -> Result<(), Error> {
        for (proof, input) in proofs.into_iter().map(DeferredProgramProof::into_parts) {
            match input.program.verify(&proof, &input.public) {
                Ok(true) => {}
                Ok(false) => return Err(Error::InvalidProgramProof),
                Err(e) => return Err(Error::MalformedProgramProof(e.to_string())),
            }
        }
        Ok(())
    }

    // TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
    // LEADER_CLAIM is retired.
    /*
    fn verify_leader_claims(proofs: Vec<DeferredLeaderClaim>) -> Result<(), Error> {
        if proofs.is_empty() {
            return Ok(());
        }

        let proof_parts = proofs
            .into_iter()
            .map(DeferredLeaderClaim::into_parts)
            .collect::<Vec<_>>();
        match lb_poc::batch_verify(proof_parts.as_slice()) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::InvalidLeaderClaimProofs),
            Err(e) => Err(Error::MalformedLeaderClaimProof(format!("{e:?}"))),
        }
    }
    */

    // TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
    /*
    #[cfg(any(test, feature = "unsafe-test-functions"))]
    #[must_use]
    pub fn zk_sigs(&self) -> &[DeferredZkSig] {
        &self.zk_sigs
    }
    */

    // TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
    // LEADER_CLAIM is retired.
    /*
    #[cfg(any(test, feature = "unsafe-test-functions"))]
    #[must_use]
    pub fn leader_claims(&self) -> &[DeferredLeaderClaim] {
        &self.leader_claims
    }
    */

    #[cfg(any(test, feature = "unsafe-test-functions"))]
    #[must_use]
    pub fn zk_transfers(&self) -> &[DeferredZkTransfer] {
        &self.zk_transfers
    }

    #[cfg(any(test, feature = "unsafe-test-functions"))]
    #[must_use]
    pub fn program_spends(&self) -> &[DeferredZkProgramSpend] {
        &self.program_spends
    }

    #[cfg(any(test, feature = "unsafe-test-functions"))]
    #[must_use]
    pub fn program_proofs(&self) -> &[DeferredProgramProof] {
        &self.program_proofs
    }
}

impl<Proof: DeferrableProof> FromIterator<Proof> for DeferredProofs {
    fn from_iter<T: IntoIterator<Item = Proof>>(iter: T) -> Self {
        let mut batch = Self::new();
        for item in iter {
            item.defer_into(&mut batch);
        }
        batch
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
    // #[error("deferred ZkSignatures are invalid")]
    // InvalidZkSignatures,
    // #[error("deferred ZkSignature is malformed: {0}")]
    // MalformedZkSignature(String),
    // TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
    // LEADER_CLAIM is retired.
    // #[error("deferred leader claim proofs are invalid")]
    // InvalidLeaderClaimProofs,
    // #[error("deferred leader claim proof is malformed: {0}")]
    // MalformedLeaderClaimProof(String),
    #[error("deferred ZkTransfer proofs are invalid")]
    InvalidZkTransfers,
    #[error("deferred ZkTransfer proof is malformed: {0}")]
    MalformedZkTransfer(String),
    #[error("deferred ZkProgramSpend proofs are invalid")]
    InvalidProgramSpends,
    #[error("deferred ZkProgramSpend proof is malformed: {0}")]
    MalformedProgramSpend(String),
    #[error("a deferred program proof is invalid")]
    InvalidProgramProof,
    #[error("a deferred program proof is malformed: {0}")]
    MalformedProgramProof(String),
}

#[cfg(test)]
pub mod test_utils {
    use super::{DeferrableProof, DeferredProofs, Error};

    pub fn batch_verify(proof: impl DeferrableProof) -> Result<(), Error> {
        DeferredProofs::from_iter([proof]).verify()
    }
}

#[cfg(test)]
mod tests {
    use lb_groth16::{
        Field as _,
        test_utils::{three_input_prove, three_input_setup},
    };

    // TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
    // use lb_key_management_system_keys::keys::{ZkKey, public_inputs_from_pks};
    // TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
    // LEADER_CLAIM is retired.
    // use lb_mmr::MerkleMountainRange;
    // TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
    // use num_bigint::BigUint;
    use super::*;
    use crate::mantle::programs::ProgramVerificationKey;
    // TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
    // LEADER_CLAIM is retired.
    /*
    use crate::{
        crypto::ZkHasher,
        mantle::ops::leader_claim::{RewardsRoot, VoucherCm, VoucherNullifier, VoucherSecret},
        proofs::leader_claim_proof::{
            Groth16LeaderClaimProof, LeaderClaimPrivate, LeaderClaimPublic,
        },
    };
    */

    #[test]
    fn verify_accepts_empty_batch() {
        DeferredProofs::new().verify().expect("must succeed");
    }

    // TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
    /*
    #[test]
    #[ignore = "TODO(RFC-480 M5): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    fn verify_accepts_batch_of_valid_zk_signatures() {
        [valid_zk_sig(7), valid_zk_sig(8), valid_zk_sig(9)]
            .into_iter()
            .collect::<DeferredProofs>()
            .verify()
            .expect("must succeed");
    }

    #[test]
    #[ignore = "TODO(RFC-480 M5): proves a ZkSignature or a PoC, both circuits are retired in circuits v0.6.0"]
    fn verify_rejects_batch_containing_invalid_zk_signature() {
        let err = [valid_zk_sig(7), invalid_zk_sig(8), valid_zk_sig(9)]
            .into_iter()
            .collect::<DeferredProofs>()
            .verify()
            .unwrap_err();
        assert!(matches!(err, Error::InvalidZkSignatures));
    }
    */

    #[test]
    fn verify_accepts_batch_of_valid_zk_transfers() {
        [zk_transfer(Fr::ONE), zk_transfer(Fr::from(2u64))]
            .into_iter()
            .collect::<DeferredProofs>()
            .verify()
            .expect("must succeed");
    }

    #[test]
    fn verify_rejects_batch_containing_invalid_zk_transfer() {
        let err = [
            zk_transfer(Fr::ONE),
            zk_transfer_checked_against_another_msg(),
        ]
        .into_iter()
        .collect::<DeferredProofs>()
        .verify()
        .unwrap_err();
        assert!(matches!(err, Error::InvalidZkTransfers));
    }

    #[test]
    fn verify_accepts_valid_program_spends_and_program_proofs() {
        let mut batch = DeferredProofs::new();
        program_spend(Fr::ONE).defer_into(&mut batch);
        program_spend(Fr::from(2u64)).defer_into(&mut batch);
        program_proof(1, 1).defer_into(&mut batch);
        batch.verify().expect("every proof is valid");
    }

    #[test]
    fn verify_rejects_a_program_spend_checked_against_another_paid_value() {
        let (proof, inputs) = program_spend(Fr::ONE).into_parts();
        let tampered = DeferredZkProgramSpend::new(
            proof,
            ZkProgramSpendVerifierInput {
                paid_value: Fr::ONE.into(),
                ..inputs
            },
        );
        assert!(matches!(
            std::iter::once(tampered)
                .collect::<DeferredProofs>()
                .verify(),
            Err(Error::InvalidProgramSpends)
        ));
    }

    #[test]
    fn verify_rejects_a_program_proof_under_another_key() {
        assert!(matches!(
            std::iter::once(program_proof(1, 2))
                .collect::<DeferredProofs>()
                .verify(),
            Err(Error::InvalidProgramProof)
        ));
    }

    // TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
    // LEADER_CLAIM is retired.
    /*
    #[test]
    fn verify_accepts_batch_of_valid_leader_claims() {
        [valid_leader_claim(7), valid_leader_claim(8)]
            .into_iter()
            .collect::<DeferredProofs>()
            .verify()
            .expect("must succeed");
    }

    #[test]
    fn verify_rejects_batch_containing_invalid_leader_claim() {
        let err = [valid_leader_claim(7), invalid_leader_claim(8)]
            .into_iter()
            .collect::<DeferredProofs>()
            .verify()
            .unwrap_err();
        assert!(matches!(err, Error::InvalidLeaderClaimProofs));
    }

    #[test]
    fn verify_rejects_invalid_leader_claim_alongside_valid_zk_signature() {
        let mut batch = DeferredProofs::new();
        valid_zk_sig(7).defer_into(&mut batch);
        invalid_leader_claim(8).defer_into(&mut batch);

        let err = batch.verify().unwrap_err();
        assert!(matches!(err, Error::InvalidLeaderClaimProofs));
    }
    */

    // TODO(RFC-480): no Operation is proven by a ZkSignature since M5.
    /*
    fn valid_zk_sig(message: u64) -> DeferredZkSig {
        zk_sig(message, message)
    }

    fn invalid_zk_sig(message: u64) -> DeferredZkSig {
        zk_sig(message + 1, message)
    }

    /// Signs `msg`, but pairs the proof with the public inputs the
    /// verifier checks it against: those of `msg_for_input`.
    /// If `msg != msg_for_input`, an invalid proof will be produced.
    fn zk_sig(msg: u64, msg_for_input: u64) -> DeferredZkSig {
        let key = ZkKey::from(BigUint::from(1u8));
        let signature = ZkKey::multi_sign(std::slice::from_ref(&key), &Fr::from(msg)).unwrap();
        let inputs =
            public_inputs_from_pks(Fr::from(msg_for_input).into(), &[key.to_public_key()]).unwrap();

        DeferredZkSig::new(*signature.as_proof(), inputs)
    }
    */

    fn zk_transfer(msg: Fr) -> DeferredZkTransfer {
        let mut data = lb_zktransfer::test_utils::sample_witness_data();
        data.chain.msg = msg;
        let (proof, inputs) = lb_zktransfer::prove(data.into()).expect("the sample witness proves");
        DeferredZkTransfer::new(proof, inputs)
    }

    fn zk_transfer_checked_against_another_msg() -> DeferredZkTransfer {
        let (proof, inputs) = zk_transfer(Fr::ONE).into_parts();
        DeferredZkTransfer::new(
            proof,
            ZkTransferVerifierInput {
                msg: Fr::from(2u64).into(),
                ..inputs
            },
        )
    }

    fn program_spend(msg: Fr) -> DeferredZkProgramSpend {
        let mut data = lb_program_spend::test_utils::sample_witness_data();
        data.chain.msg = msg;
        let (proof, inputs) =
            lb_program_spend::prove(data.into()).expect("the sample witness proves");
        DeferredZkProgramSpend::new(proof, inputs)
    }

    fn program_proof(key_seed: u64, verified_under: u64) -> DeferredProgramProof {
        let (_, proving_key) = three_input_setup(key_seed);
        let (key, _) = three_input_setup(verified_under);
        let program =
            RegisteredProgram::new(ProgramVerificationKey::try_from(&key).unwrap()).unwrap();
        let public = [Fr::from(1u64), Fr::from(2u64), Fr::from(3u64)];
        let proof =
            CompressedGroth16Proof::try_from(&three_input_prove(&proving_key, public, 4)).unwrap();
        DeferredProgramProof::new(proof, ProgramProofInput { program, public })
    }

    // TODO(RFC-480): the PoC circuit is retired in circuits v0.6.0 and
    // LEADER_CLAIM is retired.
    /*
    fn valid_leader_claim(voucher: u64) -> DeferredLeaderClaim {
        leader_claim(voucher, voucher)
    }

    fn invalid_leader_claim(voucher: u64) -> DeferredLeaderClaim {
        leader_claim(voucher + 1, voucher)
    }

    /// Proves a claim over the voucher of `secret`, but pairs the proof
    /// with the nullifier the verifier checks it against: that of
    /// `secret_for_input`.
    /// If `secret != secret_for_input`, an invalid proof will be produced.
    fn leader_claim(secret: u64, secret_for_input: u64) -> DeferredLeaderClaim {
        let voucher_secret = VoucherSecret::from(Fr::from(secret));
        let (mmr, voucher_path) = MerkleMountainRange::<VoucherCm, ZkHasher>::new()
            .push_with_paths(VoucherCm::from_secret(voucher_secret), &mut [])
            .expect("MMR shouldn't be full");
        let voucher_root = RewardsRoot::from(mmr.frontier_root());
        let tx_hash = Fr::from(11u64);
        let proof = Groth16LeaderClaimProof::prove(
            LeaderClaimPrivate::try_new(
                LeaderClaimPublic::new(
                    VoucherNullifier::from_secret(voucher_secret).into(),
                    voucher_root.into(),
                    tx_hash,
                ),
                &voucher_path,
                voucher_secret,
            )
            .expect("voucher path should match the PoC circuit height"),
        )
        .expect("proof generation should succeed");

        DeferredLeaderClaim::new(
            *proof.proof(),
            PoCVerifierInput::new(
                VoucherNullifier::from_secret(VoucherSecret::from(Fr::from(secret_for_input)))
                    .into(),
                voucher_root.into(),
                tx_hash,
            ),
        )
    }
    */
}
