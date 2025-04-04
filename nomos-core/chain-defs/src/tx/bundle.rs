use bytes::{Bytes, BytesMut};
use risc0_zkvm::Prover;

use super::Error;
use crate::{
    proofs::{balance::BalanceProof, covenant::CovenantProof, ptx::PtxProof},
    wire,
};
#[derive(Debug, Clone)]
pub struct Bundle {
    cm_roots: Vec<[u8; 32]>,
    bundle: cl::Bundle,
    //TODO: these can be pruned once validated
    // prove knowledge of input notes and well-formedness of outputs
    ptx_proofs: Vec<PtxProof>,
    // prove covenant constraints (one for each input)
    covenant_proofs: Vec<Vec<CovenantProof>>,
    // prove bundle balance
    balance_proof: BalanceProof,
}

impl Bundle {
    #[must_use]
    pub const fn bundle(&self) -> &cl::Bundle {
        &self.bundle
    }

    #[expect(
        clippy::missing_const_for_fn,
        reason = "Using `Deref` which is not const."
    )]
    #[must_use]
    pub fn cm_roots(&self) -> &[[u8; 32]] {
        &self.cm_roots
    }

    /// Requires a x86 machine with docker installed or `RISC0_DEV_MODE=1`
    pub fn prove(
        bundle_witness: &cl::BundleWitness,
        cm_root: [u8; 32],
        covenant_proofs: Vec<Vec<CovenantProof>>,
        prover: &dyn Prover,
    ) -> Result<Self, Error> {
        if bundle_witness.partial_witnesses().len() != covenant_proofs.len()
            || covenant_proofs
                .iter()
                .zip(bundle_witness.partial_witnesses())
                .any(|(covenant_proofs, ptx)| covenant_proofs.len() != ptx.inputs.len())
        {
            return Err(Error::InvalidWitness);
        }

        let ptx_proofs = bundle_witness
            .partial_witnesses()
            .iter()
            .map(|ptx| PtxProof::prove(ptx, cm_root, prover))
            .collect::<Result<Vec<_>, _>>()?;

        let balance_proof = BalanceProof::prove(bundle_witness, prover)?;
        let bundle = bundle_witness.commit();

        Ok(Self {
            cm_roots: vec![cm_root],
            bundle,
            ptx_proofs,
            covenant_proofs,
            balance_proof,
        })
    }

    #[must_use]
    pub fn as_bytes(&self) -> Bytes {
        let mut buff = BytesMut::new();
        wire::serializer_into_buffer(&mut buff)
            .serialize_into(&self)
            .expect("MockTransaction serialization to buffer failed");
        buff.freeze()
    }
}

mod serde {
    use std::collections::HashSet;

    use nomos_proof_statements::{bundle::BundlePublic, ptx::PtxPublic};
    use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};

    use super::Bundle;
    use crate::proofs::{balance::BalanceProof, covenant::CovenantProof, ptx::PtxProof};

    #[derive(Serialize, Deserialize)]
    struct BundleInner {
        ptx_proofs: Vec<PtxProof>,
        covenant_proofs: Vec<Vec<CovenantProof>>,
        balance_proof: BalanceProof,
    }

    impl Serialize for Bundle {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            // TODO: make zero copy
            BundleInner {
                ptx_proofs: self.ptx_proofs.clone(),
                covenant_proofs: self.covenant_proofs.clone(),
                balance_proof: self.balance_proof.clone(),
            }
            .serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for Bundle {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let BundleInner {
                ptx_proofs,
                covenant_proofs,
                balance_proof,
            } = BundleInner::deserialize(deserializer)?;

            // public inputs are reconstructed from proof statements
            let mut cm_roots: HashSet<[u8; 32]> = HashSet::new();
            let mut ptxs = Vec::new();

            if ptx_proofs.len() != covenant_proofs.len() {
                return Err(D::Error::custom("Invalid number of covenant proofs"));
            }

            for (ptx_proof, covenant_proofs) in ptx_proofs.iter().zip(&covenant_proofs) {
                let PtxPublic { ptx, cm_root } =
                    ptx_proof.public_inputs().map_err(D::Error::custom)?;

                for (covenant_proof, input) in covenant_proofs.iter().zip(ptx.inputs.iter()) {
                    let covenant_public =
                        covenant_proof.public_inputs().map_err(D::Error::custom)?;
                    if covenant_public.ptx_root != ptx.root()
                        || covenant_public.nf != input.nullifier
                    {
                        return Err(D::Error::custom("Invalid covenant proof public inputs"));
                    }
                    if covenant_proof.covenant() != input.covenant {
                        return Err(D::Error::custom("Invalid covenant proof"));
                    }
                }

                ptxs.push(ptx);
                cm_roots.insert(cm_root);
            }

            let balances = ptxs.iter().map(|ptx| ptx.balance).collect();
            let bundle_public = BundlePublic { balances };
            if bundle_public != balance_proof.public_inputs().map_err(D::Error::custom)? {
                return Err(D::Error::custom(format!(
                    "Invalid balance proof public inputs {:?} != {:?}",
                    bundle_public,
                    balance_proof.public_inputs().map_err(D::Error::custom)?
                )));
            }

            let bundle = cl::Bundle::new(ptxs);

            Ok(Self {
                bundle,
                balance_proof,
                ptx_proofs,
                covenant_proofs,
                cm_roots: cm_roots.into_iter().collect(),
            })
        }
    }
}

#[cfg(test)]
mod test {
    use cl::{note::derive_unit, *};

    use super::*;

    const fn receive_utxo(
        note: cl::NoteWitness,
        nf_pk: cl::NullifierCommitment,
    ) -> cl::OutputWitness {
        cl::OutputWitness::new(note, nf_pk)
    }

    #[test]
    fn test_bundle() {
        let nmo = derive_unit("NMO");
        let mut rng = rand::thread_rng();

        let sender_nf_sk = cl::NullifierSecret::random(&mut rng);
        let sender_nf_pk = sender_nf_sk.commit();

        let recipient_nf_pk = cl::NullifierSecret::random(&mut rng).commit();

        // Assume the sender has received an unspent output from somewhere
        let mut utxo = receive_utxo(cl::NoteWitness::basic(10, nmo, &mut rng), sender_nf_pk);
        utxo.note.covenant = CovenantProof::nop_constraint();
        // a little hack: if we only have one note we can put it as the merkle root
        let cm_root = cl::merkle::leaf(&utxo.commit_note().0);

        // and wants to send 8 NMO to some recipient and return 2 NMO to itself.
        let recipient_output =
            cl::OutputWitness::new(cl::NoteWitness::basic(8, nmo, &mut rng), recipient_nf_pk);
        let change_output =
            cl::OutputWitness::new(cl::NoteWitness::basic(2, nmo, &mut rng), sender_nf_pk);

        let ptx_witness = cl::PartialTxWitness {
            inputs: vec![cl::InputWitness::from_output(utxo, sender_nf_sk, vec![])],
            outputs: vec![recipient_output, change_output],
            balance_blinding: BalanceWitness::random_blinding(&mut rng),
        };

        // ATTENTION: building a valid proof requires a x86 machine with docker
        // installed if you don't have one, you can run this test with
        // RISC0_DEV_MODE=1 or skip the test
        let prover = risc0_zkvm::default_prover();
        let no_op = CovenantProof::prove_nop(
            ptx_witness.inputs[0].nullifier(),
            ptx_witness.commit().root(),
            prover.as_ref(),
        )
        .unwrap();
        let bundle = cl::BundleWitness::new(vec![ptx_witness]);
        let bundle =
            super::Bundle::prove(&bundle, cm_root, vec![vec![no_op]], prover.as_ref()).unwrap();

        assert_eq!(
            wire::serialize(&bundle).unwrap(),
            wire::serialize(
                &wire::deserialize::<super::Bundle>(&wire::serialize(&bundle).unwrap()).unwrap()
            )
            .unwrap()
        );
    }
}
