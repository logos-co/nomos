use serde::{Deserialize, Serialize};

use crate::{partial_tx::PartialTx, BalanceWitness, PartialTxWitness};
/// The transaction bundle is a collection of partial transactions.
/// The goal in bundling transactions is to produce a set of partial
/// transactions that balance each other.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    partials: Vec<PartialTx>,
}

impl Bundle {
    #[must_use]
    pub const fn new(partials: Vec<PartialTx>) -> Self {
        Self { partials }
    }

    #[must_use]
    pub fn partial_txs(&self) -> &[PartialTx] {
        &self.partials
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleWitness {
    partials: Vec<PartialTxWitness>,
}

impl BundleWitness {
    #[must_use]
    pub const fn new(partials: Vec<PartialTxWitness>) -> Self {
        Self { partials }
    }

    #[must_use]
    pub fn balance(&self) -> BalanceWitness {
        BalanceWitness::combine(
            self.partials
                .iter()
                .map(super::partial_tx::PartialTxWitness::balance),
            [0u8; 16],
        )
    }

    #[must_use]
    pub fn partial_witnesses(&self) -> &[PartialTxWitness] {
        &self.partials
    }

    #[must_use]
    pub fn commit(&self) -> Bundle {
        Bundle {
            partials: self
                .partials
                .iter()
                .map(super::partial_tx::PartialTxWitness::commit)
                .collect(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        balance::UnitBalance,
        input::InputWitness,
        note::{derive_unit, NoteWitness},
        nullifier::NullifierSecret,
        output::OutputWitness,
    };

    #[test]
    fn test_bundle_balance() {
        let mut rng = rand::thread_rng();
        let (nmo, eth, crv) = (derive_unit("NMO"), derive_unit("ETH"), derive_unit("CRV"));

        let nf_a = NullifierSecret::random(&mut rng);
        let nf_b = NullifierSecret::random(&mut rng);
        let nf_c = NullifierSecret::random(&mut rng);

        let nmo_10_utxo = OutputWitness::new(NoteWitness::basic(10, nmo, &mut rng), nf_a.commit());
        let nmo_10_in = InputWitness::from_output(nmo_10_utxo, nf_a, vec![]);

        let eth_23_utxo = OutputWitness::new(NoteWitness::basic(23, eth, &mut rng), nf_b.commit());
        let eth_23_in = InputWitness::from_output(eth_23_utxo, nf_b, vec![]);

        let crv_4840_out =
            OutputWitness::new(NoteWitness::basic(4840, crv, &mut rng), nf_c.commit());

        let ptx_unbalanced = PartialTxWitness {
            inputs: vec![nmo_10_in, eth_23_in],
            outputs: vec![crv_4840_out],
            balance_blinding: BalanceWitness::random_blinding(&mut rng),
        };

        let bundle_witness = BundleWitness {
            partials: vec![ptx_unbalanced.clone()],
        };

        assert!(!bundle_witness.balance().is_zero());
        assert_eq!(
            bundle_witness.balance().balances,
            vec![
                UnitBalance {
                    unit: nmo,
                    pos: 0,
                    neg: 10
                },
                UnitBalance {
                    unit: eth,
                    pos: 0,
                    neg: 23
                },
                UnitBalance {
                    unit: crv,
                    pos: 4840,
                    neg: 0
                },
            ]
        );

        let crv_4840_in = InputWitness::from_output(crv_4840_out, nf_c, vec![]);
        let nmo_10_out = OutputWitness::new(
            NoteWitness::basic(10, nmo, &mut rng),
            NullifierSecret::random(&mut rng).commit(), // transferring to a random owner
        );
        let eth_23_out = OutputWitness::new(
            NoteWitness::basic(23, eth, &mut rng),
            NullifierSecret::random(&mut rng).commit(), // transferring to a random owner
        );

        let ptx_solved = PartialTxWitness {
            inputs: vec![crv_4840_in],
            outputs: vec![nmo_10_out, eth_23_out],
            balance_blinding: BalanceWitness::random_blinding(&mut rng),
        };

        let witness = BundleWitness {
            partials: vec![ptx_unbalanced, ptx_solved],
        };

        assert!(witness.balance().is_zero());
        assert_eq!(witness.balance().balances, vec![]);
    }
}
